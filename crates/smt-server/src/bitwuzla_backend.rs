//! Bitwuzla backend: translates the validated wire IR to Bitwuzla via the C API
//! (see [`crate::bitwuzla_bindings`]) and supports solve, model extraction,
//! named unsat cores (via unsat assumptions), and bit-hunt optimization.
//!
//! Cancellation uses Bitwuzla's termination callback: a scoped watcher thread
//! mirrors the [`SolveContext`] token + budget deadline into an `AtomicBool`
//! the callback polls, so `check_sat` stops when the racer wins or the budget
//! elapses.

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smt_wire::raw::{
    request_flags, tag, BinaryRequest, BlobRef, Command, ExprView, ModelBlock, ModelEntry, NodeRef,
    OptimizationValueBlock, RawNode, ScalarValue, SimplifyBlock, Sort, UnsatCoreBlock, WireError,
};

use crate::backend::{Backend, QueryResult, SolveContext};
use crate::bitwuzla_bindings as ffi;
use ffi::{Bitwuzla, BitwuzlaTerm, BitwuzlaTermManager};

#[derive(Debug, Clone, Default)]
pub struct BitwuzlaBackend;

impl Backend for BitwuzlaBackend {
    fn name(&self) -> &'static str {
        "bitwuzla"
    }

    fn handle(&self, request: &BinaryRequest) -> smt_wire::Result<QueryResult> {
        handle_inner(request, None)
    }

    fn handle_with_context(
        &self,
        request: &BinaryRequest,
        context: &SolveContext,
    ) -> smt_wire::Result<QueryResult> {
        if context.is_cancelled() {
            return Ok(QueryResult::unknown(
                "bitwuzla request cancelled before start",
            ));
        }
        handle_inner(request, Some(context))
    }
}

fn handle_inner(
    request: &BinaryRequest,
    context: Option<&SolveContext>,
) -> smt_wire::Result<QueryResult> {
    let deadline = BitwuzlaDeadline::from_budget_ms(request.envelope.budget_ms);
    match request.envelope.command {
        Command::Simplify => Ok(QueryResult::simplified(SimplifyBlock {
            expression: request.expression.clone(),
            target_node: request
                .target_ref()
                .ok_or_else(|| WireError::invalid("simplify request", "missing target_node"))?,
        })),
        Command::Solve | Command::Minimize | Command::Maximize => {
            let solver = BitwuzlaSolver::new();
            match request.envelope.command {
                Command::Solve => solve(request, &solver, context, deadline),
                Command::Minimize | Command::Maximize => {
                    optimize(request, &solver, context, deadline)
                }
                Command::Simplify => unreachable!("handled above"),
            }
        }
    }
}

/// RAII guard owning a term manager, options, and a Bitwuzla instance.
///
/// `bitwuzla_new` borrows (does not own) both the term manager and options, so
/// the instance is deleted first, then the options, then the term manager.
struct BitwuzlaSolver {
    tm: *mut BitwuzlaTermManager,
    options: *mut ffi::BitwuzlaOptions,
    bz: *mut Bitwuzla,
}

// The solver is constructed and dropped within a single `handle` call on one
// thread; the raw pointers never cross threads, so this is sound without Send.

impl BitwuzlaSolver {
    fn new() -> Self {
        // Safety: the C calls are independent of thread state; the manager and
        // options are created fresh and paired with a new instance below.
        unsafe {
            let tm = ffi::bitwuzla_term_manager_new();
            let options = ffi::bitwuzla_options_new();
            // Model generation and unsat-assumption tracking are cheap and
            // always enabled so solve/optimize/core paths work uniformly.
            ffi::bitwuzla_set_option(options, ffi::BITWUZLA_OPT_PRODUCE_MODELS, 1);
            ffi::bitwuzla_set_option(options, ffi::BITWUZLA_OPT_PRODUCE_UNSAT_ASSUMPTIONS, 1);
            let bz = ffi::bitwuzla_new(tm, options);
            Self { tm, options, bz }
        }
    }

    fn tm(&self) -> *mut BitwuzlaTermManager {
        self.tm
    }

    fn bz(&self) -> *mut Bitwuzla {
        self.bz
    }
}

impl Drop for BitwuzlaSolver {
    fn drop(&mut self) {
        // Safety: each pointer was created by the matching `_new` call above and
        // is released exactly once, in the order required by the C API
        // (instance, then options, then term manager).
        unsafe {
            if !self.bz.is_null() {
                ffi::bitwuzla_delete(self.bz);
            }
            if !self.options.is_null() {
                ffi::bitwuzla_options_delete(self.options);
            }
            if !self.tm.is_null() {
                ffi::bitwuzla_term_manager_delete(self.tm);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct BitwuzlaDeadline {
    deadline: Option<Instant>,
}

impl BitwuzlaDeadline {
    fn from_budget_ms(budget_ms: u32) -> Self {
        let deadline =
            (budget_ms != 0).then(|| Instant::now() + Duration::from_millis(u64::from(budget_ms)));
        Self { deadline }
    }

    fn elapsed(self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

#[derive(Clone)]
struct BitwuzlaVariable {
    node_ref: NodeRef,
    sort: Sort,
    width: u32,
}

struct BitwuzlaTranslation {
    tm: *mut BitwuzlaTermManager,
    bool_sort: ffi::BitwuzlaSort,
    bv_sorts: HashMap<u32, ffi::BitwuzlaSort>,
    bvs: Vec<Option<BitwuzlaTerm>>,
    bools: Vec<Option<BitwuzlaTerm>>,
    variables: Vec<BitwuzlaVariable>,
}

impl BitwuzlaTranslation {
    fn bv_sort(&mut self, width: u32) -> ffi::BitwuzlaSort {
        *self
            .bv_sorts
            .entry(width)
            .or_insert_with(|| unsafe { ffi::bitwuzla_mk_bv_sort(self.tm, width as u64) })
    }

    fn bv(&self, reference: NodeRef) -> smt_wire::Result<BitwuzlaTerm> {
        if !reference.is_bv() {
            return Err(WireError::invalid(
                "bitwuzla translation",
                "expected BV reference",
            ));
        }
        self.bvs
            .get(reference.index() as usize)
            .and_then(|opt| *opt)
            .ok_or_else(|| WireError::invalid("bitwuzla translation", "missing BV term"))
    }

    fn bool(&self, reference: NodeRef) -> smt_wire::Result<BitwuzlaTerm> {
        if !reference.is_bool() {
            return Err(WireError::invalid(
                "bitwuzla translation",
                "expected Bool reference",
            ));
        }
        self.bools
            .get(reference.index() as usize)
            .and_then(|opt| *opt)
            .ok_or_else(|| WireError::invalid("bitwuzla translation", "missing Bool term"))
    }
}

fn translate(
    request: &BinaryRequest,
    tm: *mut BitwuzlaTermManager,
) -> smt_wire::Result<BitwuzlaTranslation> {
    let expr = request.expression_view()?;
    let bool_sort = unsafe { ffi::bitwuzla_mk_bool_sort(tm) };
    let mut out = BitwuzlaTranslation {
        tm,
        bool_sort,
        bv_sorts: HashMap::new(),
        bvs: vec![None; expr.node_count() as usize],
        bools: vec![None; expr.node_count() as usize],
        variables: Vec::new(),
    };
    let mut bv_vars: HashMap<(String, u32), BitwuzlaTerm> = HashMap::new();
    let mut bool_vars: HashMap<String, BitwuzlaTerm> = HashMap::new();

    for index in 0..expr.node_count() {
        let node = expr.node(index)?;
        match node.tag {
            tag::BV_VAR => {
                let name = expr
                    .blob_str(BlobRef::from_payload(node.payload), "BV variable")?
                    .to_owned();
                let term = if let Some(&existing) = bv_vars.get(&(name.clone(), node.width)) {
                    existing
                } else {
                    let sort = out.bv_sort(node.width);
                    let symbol = CString::new(format!("bv_{index}"))
                        .map_err(|_| WireError::invalid("BV variable", "symbol contains nul"))?;
                    let term = unsafe { ffi::bitwuzla_mk_const(tm, sort, symbol.as_ptr()) };
                    bv_vars.insert((name, node.width), term);
                    term
                };
                out.variables.push(BitwuzlaVariable {
                    node_ref: NodeRef::bv(index)?,
                    sort: Sort::Bv,
                    width: node.width,
                });
                out.bvs[index as usize] = Some(term);
            }
            tag::BV_CONST => {
                out.bvs[index as usize] = Some(bitwuzla_bv_const(
                    &expr,
                    &mut out,
                    node.width,
                    node.payload,
                )?);
            }
            tag::BV_NOT => {
                let x = child_bv(&expr, &out, &node, 0)?;
                out.bvs[index as usize] = Some(mk1(tm, ffi::BITWUZLA_KIND_BV_NOT, x));
            }
            tag::BV_NEG => {
                let x = child_bv(&expr, &out, &node, 0)?;
                out.bvs[index as usize] = Some(mk1(tm, ffi::BITWUZLA_KIND_BV_NEG, x));
            }
            tag::BV_AND => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_AND, a, b));
            }
            tag::BV_OR => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_OR, a, b));
            }
            tag::BV_XOR => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_XOR, a, b));
            }
            tag::BV_ADD => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_ADD, a, b));
            }
            tag::BV_SUB => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SUB, a, b));
            }
            tag::BV_MUL => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_MUL, a, b));
            }
            tag::BV_UDIV => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_UDIV, a, b));
            }
            tag::BV_UREM => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_UREM, a, b));
            }
            tag::BV_SDIV => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SDIV, a, b));
            }
            tag::BV_SREM => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SREM, a, b));
            }
            tag::BV_SMOD => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SMOD, a, b));
            }
            tag::BV_SHL => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SHL, a, b));
            }
            tag::BV_LSHR => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SHR, a, b));
            }
            tag::BV_ASHR => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_ASHR, a, b));
            }
            tag::BV_EXTRACT => {
                let x = child_bv(&expr, &out, &node, 0)?;
                out.bvs[index as usize] = Some(unsafe {
                    ffi::bitwuzla_mk_term1_indexed2(
                        tm,
                        ffi::BITWUZLA_KIND_BV_EXTRACT,
                        x,
                        u64::from(node.aux_hi),
                        u64::from(node.aux_lo),
                    )
                });
            }
            tag::BV_CONCAT => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bvs[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_CONCAT, a, b));
            }
            tag::BV_ZEXT => {
                let x = child_bv(&expr, &out, &node, 0)?;
                out.bvs[index as usize] = Some(unsafe {
                    ffi::bitwuzla_mk_term1_indexed1(
                        tm,
                        ffi::BITWUZLA_KIND_BV_ZERO_EXTEND,
                        x,
                        u64::from(node.aux_hi),
                    )
                });
            }
            tag::BV_SEXT => {
                let x = child_bv(&expr, &out, &node, 0)?;
                out.bvs[index as usize] = Some(unsafe {
                    ffi::bitwuzla_mk_term1_indexed1(
                        tm,
                        ffi::BITWUZLA_KIND_BV_SIGN_EXTEND,
                        x,
                        u64::from(node.aux_hi),
                    )
                });
            }
            tag::BV_ITE => {
                let c = child_bool(&expr, &out, &node, 0)?;
                let t = child_bv(&expr, &out, &node, 1)?;
                let e = child_bv(&expr, &out, &node, 2)?;
                out.bvs[index as usize] = Some(mk3(tm, ffi::BITWUZLA_KIND_ITE, c, t, e));
            }
            tag::BV_SELECT => {
                let pairs = u32::from(node.aux_hi);
                let mut result = child_bv(&expr, &out, &node, pairs * 2)?;
                for pair in (0..pairs).rev() {
                    let selector = child_bool(&expr, &out, &node, pair * 2)?;
                    let value = child_bv(&expr, &out, &node, pair * 2 + 1)?;
                    result = mk3(tm, ffi::BITWUZLA_KIND_ITE, selector, value, result);
                }
                out.bvs[index as usize] = Some(result);
            }
            tag::BOOL_TRUE => {
                out.bools[index as usize] = Some(unsafe { ffi::bitwuzla_mk_true(tm) })
            }
            tag::BOOL_FALSE => {
                out.bools[index as usize] = Some(unsafe { ffi::bitwuzla_mk_false(tm) })
            }
            tag::BOOL_VAR => {
                let name = expr
                    .blob_str(BlobRef::from_payload(node.payload), "Bool variable")?
                    .to_owned();
                let term = if let Some(&existing) = bool_vars.get(&name) {
                    existing
                } else {
                    let symbol = CString::new(format!("bool_{index}"))
                        .map_err(|_| WireError::invalid("Bool variable", "symbol contains nul"))?;
                    let term =
                        unsafe { ffi::bitwuzla_mk_const(tm, out.bool_sort, symbol.as_ptr()) };
                    bool_vars.insert(name, term);
                    term
                };
                out.variables.push(BitwuzlaVariable {
                    node_ref: NodeRef::bool(index)?,
                    sort: Sort::Bool,
                    width: 0,
                });
                out.bools[index as usize] = Some(term);
            }
            tag::BOOL_NOT => {
                let x = child_bool(&expr, &out, &node, 0)?;
                out.bools[index as usize] = Some(mk1(tm, ffi::BITWUZLA_KIND_NOT, x));
            }
            tag::BOOL_AND => {
                let (a, b) = child_bool2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_AND, a, b));
            }
            tag::BOOL_OR => {
                let (a, b) = child_bool2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_OR, a, b));
            }
            tag::BOOL_IMPLIES => {
                let (a, b) = child_bool2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_IMPLIES, a, b));
            }
            tag::BV_EQ => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_EQUAL, a, b));
            }
            tag::BV_ULT => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_ULT, a, b));
            }
            tag::BV_ULE => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_ULE, a, b));
            }
            tag::BV_SLT => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SLT, a, b));
            }
            tag::BV_SLE => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] = Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SLE, a, b));
            }
            tag::UADD_OVF => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] =
                    Some(mk2(tm, ffi::BITWUZLA_KIND_BV_UADD_OVERFLOW, a, b));
            }
            tag::SADD_OVF => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] =
                    Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SADD_OVERFLOW, a, b));
            }
            tag::USUB_OVF => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] =
                    Some(mk2(tm, ffi::BITWUZLA_KIND_BV_USUB_OVERFLOW, a, b));
            }
            tag::SSUB_OVF => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] =
                    Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SSUB_OVERFLOW, a, b));
            }
            tag::UMUL_OVF => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] =
                    Some(mk2(tm, ffi::BITWUZLA_KIND_BV_UMUL_OVERFLOW, a, b));
            }
            tag::SMUL_OVF => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] =
                    Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SMUL_OVERFLOW, a, b));
            }
            tag::NEG_OVF => {
                let a = child_bv(&expr, &out, &node, 0)?;
                out.bools[index as usize] = Some(mk1(tm, ffi::BITWUZLA_KIND_BV_NEG_OVERFLOW, a));
            }
            tag::SDIV_OVF => {
                let (a, b) = child_bv2(&expr, &out, &node)?;
                out.bools[index as usize] =
                    Some(mk2(tm, ffi::BITWUZLA_KIND_BV_SDIV_OVERFLOW, a, b));
            }
            other => {
                return Err(WireError::invalid(
                    "bitwuzla translation",
                    format!("unknown tag {other}"),
                ))
            }
        }
    }
    Ok(out)
}

fn solve(
    request: &BinaryRequest,
    solver: &BitwuzlaSolver,
    context: Option<&SolveContext>,
    deadline: BitwuzlaDeadline,
) -> smt_wire::Result<QueryResult> {
    let expr = request.expression_view()?;
    let translation = translate(request, solver.tm())?;
    let bz = solver.bz();
    let want_model = (request.envelope.flags & request_flags::WANT_MODEL) != 0;
    let want_core = (request.envelope.flags & request_flags::WANT_CORE) != 0;
    let named_count = request.named_assertion_refs.len();

    // Named assertions become assumptions (so `get_unsat_assumptions` can
    // identify which are in the core); unnamed assertions stay hard.
    let mut named: Vec<(BitwuzlaTerm, String)> = Vec::new();
    let mut assumptions: Vec<BitwuzlaTerm> = Vec::new();
    for (index, root) in request.assertion_roots.iter().enumerate() {
        let assertion = translation.bool(*root)?;
        if index < named_count {
            let name = expr
                .blob_str(request.named_assertion_refs[index], "named assertion")?
                .to_owned();
            assumptions.push(assertion);
            named.push((assertion, name));
        } else {
            // Safety: `bz` is a valid, non-null Bitwuzla instance.
            unsafe { ffi::bitwuzla_assert(bz, assertion) };
        }
    }
    for root in &request.assumption_roots {
        assumptions.push(translation.bool(*root)?);
    }

    let result = check_with_cancellation(bz, &mut assumptions, context, deadline);
    match result {
        ffi::BITWUZLA_SAT => {
            let model = if want_model {
                Some(build_model(bz, &translation)?)
            } else {
                None
            };
            Ok(QueryResult::sat(model))
        }
        ffi::BITWUZLA_UNSAT => {
            let core = if want_core {
                Some(build_core(bz, &named))
            } else {
                None
            };
            Ok(QueryResult::unsat(core))
        }
        _ => Ok(QueryResult::unknown("bitwuzla returned unknown".to_owned())),
    }
}

fn optimize(
    request: &BinaryRequest,
    solver: &BitwuzlaSolver,
    context: Option<&SolveContext>,
    deadline: BitwuzlaDeadline,
) -> smt_wire::Result<QueryResult> {
    let expr = request.expression_view()?;
    let mut translation = translate(request, solver.tm())?;
    let tm = solver.tm();
    let bz = solver.bz();
    let want_model = (request.envelope.flags & request_flags::WANT_MODEL) != 0;
    let signed = (request.envelope.flags & request_flags::SIGNED) != 0;
    let minimize = request.envelope.command == Command::Minimize;

    // All assertions are hard for optimization; the assumption roots seed the
    // per-bit assumption list.
    for root in &request.assertion_roots {
        // Safety: `bz` is a valid Bitwuzla instance.
        unsafe { ffi::bitwuzla_assert(bz, translation.bool(*root)?) };
    }
    let mut fixed: Vec<BitwuzlaTerm> = request
        .assumption_roots
        .iter()
        .map(|root| translation.bool(*root))
        .collect::<smt_wire::Result<Vec<_>>>()?;

    let result = check_with_cancellation(bz, &mut fixed, context, deadline);
    if result == ffi::BITWUZLA_UNSAT {
        return Ok(QueryResult::unsat(None));
    }
    if result != ffi::BITWUZLA_SAT {
        return Ok(QueryResult::unknown("bitwuzla returned unknown".to_owned()));
    }

    let target_ref = request
        .target_ref()
        .ok_or_else(|| WireError::invalid("optimization", "missing target"))?;
    let target = translation.bv(target_ref)?;
    let width = expr.node(target_ref.index())?.width;
    let one_bit = unsafe { ffi::bitwuzla_mk_bv_value_uint64(tm, translation.bv_sort(1), 1) };
    let mut optimum = vec![0u8; (width as usize).div_ceil(8)];

    for bit in (0..width).rev() {
        let prefer_one = match (signed, minimize, bit == width - 1) {
            (false, true, _) => false,
            (false, false, _) => true,
            (true, true, true) => true,
            (true, true, false) => false,
            (true, false, true) => false,
            (true, false, false) => true,
        };
        let bit_is_one = {
            let extracted = unsafe {
                ffi::bitwuzla_mk_term1_indexed2(
                    tm,
                    ffi::BITWUZLA_KIND_BV_EXTRACT,
                    target,
                    u64::from(bit),
                    u64::from(bit),
                )
            };
            mk2(tm, ffi::BITWUZLA_KIND_EQUAL, extracted, one_bit)
        };
        let first_try = if prefer_one {
            bit_is_one
        } else {
            mk1(tm, ffi::BITWUZLA_KIND_NOT, bit_is_one)
        };
        fixed.push(first_try);
        match check_with_cancellation(bz, &mut fixed, context, deadline) {
            ffi::BITWUZLA_SAT => {
                if prefer_one {
                    set_bit(&mut optimum, bit);
                }
            }
            ffi::BITWUZLA_UNSAT => {
                let last = fixed.len() - 1;
                fixed[last] = if prefer_one {
                    mk1(tm, ffi::BITWUZLA_KIND_NOT, bit_is_one)
                } else {
                    bit_is_one
                };
                if !prefer_one {
                    set_bit(&mut optimum, bit);
                }
            }
            _ => return Ok(QueryResult::unknown("bitwuzla returned unknown".to_owned())),
        }
    }

    match check_with_cancellation(bz, &mut fixed, context, deadline) {
        ffi::BITWUZLA_SAT => {}
        ffi::BITWUZLA_UNSAT => return Ok(QueryResult::unsat(None)),
        _ => return Ok(QueryResult::unknown("bitwuzla returned unknown".to_owned())),
    }

    let model = if want_model {
        Some(build_model(bz, &translation)?)
    } else {
        None
    };
    Ok(QueryResult::sat_optimization(OptimizationValueBlock {
        optimum: ScalarValue::bv(width, optimum)?,
        model,
    }))
}

fn build_model(
    bz: *mut Bitwuzla,
    translation: &BitwuzlaTranslation,
) -> smt_wire::Result<ModelBlock> {
    let mut entries = Vec::with_capacity(translation.variables.len());
    for variable in &translation.variables {
        let value = match variable.sort {
            Sort::Bool => {
                let term = translation.bool(variable.node_ref)?;
                // Safety: `bz` is valid and model generation is enabled.
                let value = unsafe { ffi::bitwuzla_get_value(bz, term) };
                ScalarValue::bool(unsafe { ffi::bitwuzla_term_value_get_bool(value) })
            }
            Sort::Bv => {
                let term = translation.bv(variable.node_ref)?;
                // Safety: `bz` is valid and model generation is enabled.
                let value = unsafe { ffi::bitwuzla_get_value(bz, term) };
                bv_model_value(variable.width, value)?
            }
        };
        entries.push(ModelEntry {
            node_ref: variable.node_ref,
            value,
        });
    }
    Ok(ModelBlock { entries })
}

/// Maps Bitwuzla's unsat assumptions back to the originating named assertions.
fn build_core(bz: *mut Bitwuzla, named: &[(BitwuzlaTerm, String)]) -> UnsatCoreBlock {
    if named.is_empty() {
        return UnsatCoreBlock { names: Vec::new() };
    }
    let mut size: usize = 0;
    // Safety: `bz` is valid and the last check returned UNSAT with unsat-assumption
    // tracking enabled.
    let ptr = unsafe { ffi::bitwuzla_get_unsat_assumptions(bz, &mut size) };
    let failed: HashSet<BitwuzlaTerm> = if ptr.is_null() || size == 0 {
        HashSet::new()
    } else {
        // Safety: `ptr` points to `size` contiguous `BitwuzlaTerm` values valid
        // until the next Bitwuzla call, and we copy them out immediately.
        unsafe { std::slice::from_raw_parts(ptr, size) }
            .iter()
            .copied()
            .collect()
    };
    let names = named
        .iter()
        .filter(|(term, _)| failed.contains(term))
        .map(|(_, name)| name.clone())
        .collect();
    UnsatCoreBlock { names }
}

/// Runs `check_sat` / `check_sat_assuming` with cooperative cancellation.
///
/// A termination callback is installed that polls a shared `AtomicBool`; a
/// scoped watcher thread sets it when the [`SolveContext`] is cancelled or the
/// budget deadline elapses, prompting Bitwuzla to return `UNKNOWN`.
fn check_with_cancellation(
    bz: *mut Bitwuzla,
    assumptions: &mut [BitwuzlaTerm],
    context: Option<&SolveContext>,
    deadline: BitwuzlaDeadline,
) -> ffi::BitwuzlaResult {
    if deadline.elapsed() {
        return ffi::BITWUZLA_UNKNOWN;
    }
    let flag = Arc::new(AtomicBool::new(false));
    // Safety: `bz` is valid; the callback only reads through `state`, which
    // points at the `AtomicBool` owned by `flag` (alive for this whole call).
    unsafe {
        ffi::bitwuzla_set_termination_callback(
            bz,
            Some(term_callback),
            Arc::as_ptr(&flag) as *mut std::ffi::c_void,
        );
    }

    if context.is_none() && deadline.deadline.is_none() {
        return unsafe { run_check(bz, assumptions) };
    }

    let done = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        let flag = flag.clone();
        let done_w = done.clone();
        scope.spawn(move || {
            while !done_w.load(Ordering::Acquire) {
                let cancelled = context.is_some_and(|c| c.is_cancelled()) || deadline.elapsed();
                if cancelled {
                    flag.store(true, Ordering::Release);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let result = unsafe { run_check(bz, assumptions) };
        done.store(true, Ordering::Release);
        result
    })
}

unsafe fn run_check(bz: *mut Bitwuzla, assumptions: &mut [BitwuzlaTerm]) -> ffi::BitwuzlaResult {
    if assumptions.is_empty() {
        ffi::bitwuzla_check_sat(bz)
    } else {
        ffi::bitwuzla_check_sat_assuming(bz, assumptions.len() as u32, assumptions.as_mut_ptr())
    }
}

unsafe extern "C" fn term_callback(state: *mut std::ffi::c_void) -> i32 {
    if state.is_null() {
        return 0;
    }
    // Safety: `state` points at an `AtomicBool` owned by an `Arc` that outlives
    // the `check_sat` call during which this callback runs.
    let flag = &*(state as *const AtomicBool);
    i32::from(flag.load(Ordering::Acquire))
}

fn mk1(tm: *mut BitwuzlaTermManager, kind: ffi::BitwuzlaKind, arg: BitwuzlaTerm) -> BitwuzlaTerm {
    unsafe { ffi::bitwuzla_mk_term1(tm, kind, arg) }
}

fn mk2(
    tm: *mut BitwuzlaTermManager,
    kind: ffi::BitwuzlaKind,
    a: BitwuzlaTerm,
    b: BitwuzlaTerm,
) -> BitwuzlaTerm {
    unsafe { ffi::bitwuzla_mk_term2(tm, kind, a, b) }
}

fn mk3(
    tm: *mut BitwuzlaTermManager,
    kind: ffi::BitwuzlaKind,
    a: BitwuzlaTerm,
    b: BitwuzlaTerm,
    c: BitwuzlaTerm,
) -> BitwuzlaTerm {
    unsafe { ffi::bitwuzla_mk_term3(tm, kind, a, b, c) }
}

fn child_bv(
    expr: &ExprView<'_>,
    translation: &BitwuzlaTranslation,
    node: &RawNode,
    offset: u32,
) -> smt_wire::Result<BitwuzlaTerm> {
    translation.bv(expr.child_ref(node.children + offset)?)
}

fn child_bool(
    expr: &ExprView<'_>,
    translation: &BitwuzlaTranslation,
    node: &RawNode,
    offset: u32,
) -> smt_wire::Result<BitwuzlaTerm> {
    translation.bool(expr.child_ref(node.children + offset)?)
}

fn child_bv2(
    expr: &ExprView<'_>,
    translation: &BitwuzlaTranslation,
    node: &RawNode,
) -> smt_wire::Result<(BitwuzlaTerm, BitwuzlaTerm)> {
    Ok((
        child_bv(expr, translation, node, 0)?,
        child_bv(expr, translation, node, 1)?,
    ))
}

fn child_bool2(
    expr: &ExprView<'_>,
    translation: &BitwuzlaTranslation,
    node: &RawNode,
) -> smt_wire::Result<(BitwuzlaTerm, BitwuzlaTerm)> {
    Ok((
        child_bool(expr, translation, node, 0)?,
        child_bool(expr, translation, node, 1)?,
    ))
}

fn bitwuzla_bv_const(
    expr: &ExprView<'_>,
    out: &mut BitwuzlaTranslation,
    width: u32,
    payload: u64,
) -> smt_wire::Result<BitwuzlaTerm> {
    let tm = out.tm;
    let sort = out.bv_sort(width);
    if width <= 64 {
        // Safety: `tm` and `sort` are valid.
        Ok(unsafe { ffi::bitwuzla_mk_bv_value_uint64(tm, sort, payload) })
    } else {
        let bytes = expr.blob_ref(BlobRef::from_payload(payload))?;
        // Bitwuzla parses base-2 values MSB-first.
        let mut bits = String::with_capacity(width as usize);
        for bit in (0..width).rev() {
            let byte = bytes[(bit / 8) as usize];
            bits.push(if ((byte >> (bit % 8)) & 1) != 0 {
                '1'
            } else {
                '0'
            });
        }
        let value = CString::new(bits)
            .map_err(|_| WireError::invalid("BV_CONST", "wide constant contains a nul byte"))?;
        // Safety: `tm` and `sort` are valid and `value` is a nul-terminated base-2 string.
        Ok(unsafe { ffi::bitwuzla_mk_bv_value(tm, sort, value.as_ptr(), 2) })
    }
}

fn bv_model_value(width: u32, value: BitwuzlaTerm) -> smt_wire::Result<ScalarValue> {
    // Safety: `value` is a model value term with model generation enabled.
    let ptr = unsafe { ffi::bitwuzla_term_value_get_str_fmt(value, 2) };
    if ptr.is_null() {
        return Err(WireError::invalid(
            "bitwuzla model",
            "missing BV value string",
        ));
    }
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| WireError::invalid("bitwuzla model", "non-UTF-8 BV value"))?;
    let mut bytes = vec![0u8; (width as usize).div_ceil(8)];
    // The base-2 string is MSB-first, so its reversed enumeration is bit 0 (LSB) up.
    for (pos, ch) in text.chars().rev().enumerate() {
        if ch == '1' {
            set_bit(&mut bytes, pos as u32);
        }
    }
    ScalarValue::bv(width, bytes)
}

fn set_bit(bytes: &mut [u8], bit: u32) {
    bytes[(bit / 8) as usize] |= 1u8 << (bit % 8);
}
