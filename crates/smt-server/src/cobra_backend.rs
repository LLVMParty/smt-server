use std::collections::HashMap;
use std::sync::Arc;

use cobra::{
    simplify_expr, Expr as CobraExpr, Kind as CobraKind, Options as CobraOptions, ProofLevel,
    SimplifyOutcomeKind,
};
use smt_wire::raw::{
    tag, BinaryRequest, Command, ExprBuilder, ExprView, NodeRef, RawNode, SimplifyBlock, WireError,
};

use crate::backend::{Backend, QueryResult, SolveContext};

const MAX_COBRA_WIDTH: u32 = 64;

/// Simplifier backend built on [CoBRA](https://github.com/binsnake/cobra)
/// (`cobra-mba`), a worklist-driven mixed Boolean-arithmetic simplifier.
///
/// Like the Rumba backend, CoBRA only understands pure same-width bit-vector
/// MBA trees, so this backend rewrites each maximal MBA island of the target
/// expression in place and copies the structural skeleton verbatim.
///
/// CoBRA verifies candidates with finite probing plus a Lean certificate
/// replay. In the default certified mode only rewrites backed by a replayable
/// Lean certificate are adopted; islands without one are left unchanged. The
/// spot-checked mode also adopts probe-verified rewrites, which raises the
/// simplification rate dramatically but can in principle return an expression
/// that differs from the input at an unprobed point, so it should only be used
/// on non-adversarial inputs.
#[derive(Debug, Clone)]
pub struct CobraBackend {
    require_certificate: bool,
}

impl CobraBackend {
    /// Certified mode: only adopt rewrites CoBRA backs with a Lean certificate.
    pub fn new() -> Self {
        Self {
            require_certificate: true,
        }
    }

    /// Spot-checked mode: also adopt probe-verified rewrites without a
    /// certificate. Higher simplification rate, probabilistic soundness.
    pub fn spot_checked() -> Self {
        Self {
            require_certificate: false,
        }
    }
}

impl Default for CobraBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for CobraBackend {
    fn name(&self) -> &'static str {
        "cobra"
    }

    fn handle(&self, request: &BinaryRequest) -> smt_wire::Result<QueryResult> {
        match request.envelope.command {
            Command::Simplify => {
                simplify_request(request, self.require_certificate).map(QueryResult::simplified)
            }
            Command::Solve | Command::Minimize | Command::Maximize => {
                Ok(QueryResult::unknown("cobra only supports SIMPLIFY"))
            }
        }
    }

    fn handle_with_context(
        &self,
        request: &BinaryRequest,
        context: &SolveContext,
    ) -> smt_wire::Result<QueryResult> {
        if context.is_cancelled() {
            return Ok(QueryResult::unknown("cobra request cancelled before start"));
        }
        self.handle(request)
    }
}

/// Simplify the target expression by rewriting each maximal MBA island in
/// place, exactly like the Rumba backend: walk the DAG, copy the structural
/// skeleton verbatim, and hand every maximal MBA island to CoBRA -- abstracting
/// any non-MBA / wrong-width child of an island as a fresh opaque variable that
/// maps back to the recursively-simplified subtree.
fn simplify_request(
    request: &BinaryRequest,
    require_certificate: bool,
) -> smt_wire::Result<SimplifyBlock> {
    let target = request
        .target_ref()
        .ok_or_else(|| WireError::invalid("simplify request", "missing target_node"))?;
    let view = request.expression_view()?;
    let mut simplifier = IslandSimplifier::new(view, require_certificate);
    match simplifier.process(target) {
        Ok(root) => Ok(SimplifyBlock {
            expression: simplifier.builder.to_bytes()?,
            target_node: root,
        }),
        // Any structural failure falls back to returning the input unchanged, which is
        // always a sound (if unhelpful) simplification result.
        Err(_) => Ok(identity_simplify(request, target)),
    }
}

fn identity_simplify(request: &BinaryRequest, target_node: NodeRef) -> SimplifyBlock {
    SimplifyBlock {
        expression: request.expression.clone(),
        target_node,
    }
}

/// True for the bit-vector operators CoBRA understands; these are the roots of
/// MBA islands. Leaves (VAR/CONST) are copied directly; everything else is
/// structural. `BV_LSHR` by a constant amount also converts inside an island,
/// but a lone shift is not worth an island of its own.
fn is_mba_op(tag: u8) -> bool {
    matches!(
        tag,
        tag::BV_NOT
            | tag::BV_NEG
            | tag::BV_AND
            | tag::BV_OR
            | tag::BV_XOR
            | tag::BV_ADD
            | tag::BV_SUB
            | tag::BV_MUL
    )
}

/// Total island-conversion node visits allowed per request, as a multiple of
/// the input's node count (plus a floor so small requests get full attempts).
///
/// A declined maximal island is retried on each of its subtrees, and CoBRA's
/// tree conversion re-expands shared wire DAG nodes on every visit, so without
/// a shared cap an adversarial unsimplifiable expression could turn one
/// bounded request into quadratic (or, through sharing, exponential)
/// conversion work and unbounded repeat runs of CoBRA's search pipeline. Once
/// the budget is spent, remaining islands are copied structurally.
const ISLAND_BUDGET_FACTOR: usize = 8;
const ISLAND_BUDGET_FLOOR: usize = 1 << 14;

struct IslandSimplifier<'a> {
    view: ExprView<'a>,
    builder: ExprBuilder,
    memo: HashMap<NodeRef, NodeRef>,
    require_certificate: bool,
    conversion_budget: usize,
}

impl<'a> IslandSimplifier<'a> {
    fn new(view: ExprView<'a>, require_certificate: bool) -> Self {
        let conversion_budget = (view.node_count() as usize)
            .saturating_mul(ISLAND_BUDGET_FACTOR)
            .max(ISLAND_BUDGET_FLOOR);
        Self {
            view,
            builder: ExprBuilder::new(),
            memo: HashMap::new(),
            require_certificate,
            conversion_budget,
        }
    }

    fn process(&mut self, reference: NodeRef) -> smt_wire::Result<NodeRef> {
        if let Some(&cached) = self.memo.get(&reference) {
            return Ok(cached);
        }
        let result = self.process_uncached(reference)?;
        self.memo.insert(reference, result);
        Ok(result)
    }

    fn process_uncached(&mut self, reference: NodeRef) -> smt_wire::Result<NodeRef> {
        let node = self.view.node(reference.index())?;
        if reference.is_bv()
            && is_mba_op(node.tag)
            && (1..=MAX_COBRA_WIDTH).contains(&node.width)
            && self.conversion_budget > 0
        {
            if let Some(simplified) = self.try_island(reference, &node)? {
                return Ok(simplified);
            }
        }
        self.copy_node(reference, &node)
    }

    /// Convert the MBA island rooted at `reference`, simplify it with CoBRA,
    /// and lower the result back into the builder. Returns `None` (so the
    /// caller copies the node structurally instead) if CoBRA declines the
    /// island or fails to improve it under the configured proof requirement.
    fn try_island(
        &mut self,
        reference: NodeRef,
        node: &RawNode,
    ) -> smt_wire::Result<Option<NodeRef>> {
        let width = node.width;
        let mut conversion = IslandConversion::new(self.view, width, &mut self.conversion_budget);
        // A conversion failure (typically the shared budget running out) means
        // this island is skipped, not that the request is bad: the caller
        // copies the node structurally and any real structural defect
        // resurfaces there.
        let Ok(expr) = conversion.convert(reference) else {
            return Ok(None);
        };
        let vars = conversion.vars;
        let names = island_var_names(&vars);
        let options = CobraOptions {
            bitwidth: width,
            require_lean_certificate: self.require_certificate,
            ..CobraOptions::default()
        };
        // A `CobraError` (too many variables, node budget, ...) means CoBRA
        // could not handle this island, not that the request is bad: leave the
        // island alone and let the caller copy it structurally.
        let outcome = match simplify_expr(&expr, &names, options) {
            Ok(outcome) => outcome,
            Err(_) => return Ok(None),
        };
        if outcome.kind != SimplifyOutcomeKind::Simplified {
            return Ok(None);
        }
        // CoBRA's own certificate gate covers the main pipeline, but a few
        // side paths still return probe-verified results; in certified mode
        // adopt nothing weaker than a Lean-certified rewrite.
        if self.require_certificate && outcome.proof_level != ProofLevel::LeanCertified {
            return Ok(None);
        }
        let Some(simplified) = outcome.expr else {
            return Ok(None);
        };
        // The islands handed over are uniform-width, so a result using
        // width-changing operators (or unknown variables) is out of contract.
        if !is_lowerable(&simplified, vars.len()) {
            return Ok(None);
        }
        let root = self.lower(&simplified, width, &vars)?;
        Ok(Some(root))
    }

    /// Lower a simplified CoBRA expression into the builder; opaque boundary
    /// variables resolve to the recursively-simplified subtrees they
    /// abstracted.
    fn lower(
        &mut self,
        expr: &CobraExpr,
        width: u32,
        vars: &[IslandVar],
    ) -> smt_wire::Result<NodeRef> {
        let mask = mask_for_width(width);
        match expr.kind {
            CobraKind::Variable(index) => match vars.get(index as usize) {
                Some(IslandVar::Named(name)) => self.builder.bv_var(name, width),
                Some(IslandVar::Boundary(reference)) => self.process(*reference),
                None => Err(WireError::invalid(
                    "cobra expression",
                    format!("unknown CoBRA variable v{index}"),
                )),
            },
            CobraKind::Constant(value) => self.builder.bv_const(value & mask, width),
            CobraKind::Not => {
                let child = self.lower_child(expr, 0, width, vars)?;
                self.builder.bv_not(child)
            }
            CobraKind::Neg => {
                let child = self.lower_child(expr, 0, width, vars)?;
                self.builder.bv_neg(child)
            }
            CobraKind::Shr(amount) => {
                // CoBRA pins the shift amount in the node; an amount at or
                // beyond the width yields zero, matching SMT `bvlshr`.
                if u64::from(amount) >= u64::from(width) {
                    self.builder.bv_const(0, width)
                } else {
                    let child = self.lower_child(expr, 0, width, vars)?;
                    let amount = self.builder.bv_const(u64::from(amount), width)?;
                    self.builder.bv_lshr(child, amount)
                }
            }
            CobraKind::Add => self.lower_binary(expr, width, vars, ExprBuilder::bv_add),
            CobraKind::Mul => self.lower_binary(expr, width, vars, ExprBuilder::bv_mul),
            CobraKind::And => self.lower_binary(expr, width, vars, ExprBuilder::bv_and),
            CobraKind::Or => self.lower_binary(expr, width, vars, ExprBuilder::bv_or),
            CobraKind::Xor => self.lower_binary(expr, width, vars, ExprBuilder::bv_xor),
            CobraKind::ZExt(_) | CobraKind::SExt(_) | CobraKind::Trunc(_) | CobraKind::Concat => {
                Err(WireError::invalid(
                    "cobra expression",
                    "width-changing operator in uniform-width island result",
                ))
            }
        }
    }

    fn lower_child(
        &mut self,
        expr: &CobraExpr,
        index: usize,
        width: u32,
        vars: &[IslandVar],
    ) -> smt_wire::Result<NodeRef> {
        let child = expr.children.get(index).ok_or_else(|| {
            WireError::invalid("cobra expression", "operator node missing a child")
        })?;
        self.lower(child, width, vars)
    }

    fn lower_binary(
        &mut self,
        expr: &CobraExpr,
        width: u32,
        vars: &[IslandVar],
        op: fn(&mut ExprBuilder, NodeRef, NodeRef) -> smt_wire::Result<NodeRef>,
    ) -> smt_wire::Result<NodeRef> {
        let a = self.lower_child(expr, 0, width, vars)?;
        let b = self.lower_child(expr, 1, width, vars)?;
        op(&mut self.builder, a, b)
    }

    fn src_child(&self, node: &RawNode, offset: u32) -> smt_wire::Result<NodeRef> {
        let index = node
            .children
            .checked_add(offset)
            .ok_or(WireError::IntegerOverflow("child array index"))?;
        self.view.child_ref(index)
    }

    fn processed_child(&mut self, node: &RawNode, offset: u32) -> smt_wire::Result<NodeRef> {
        let child = self.src_child(node, offset)?;
        self.process(child)
    }

    /// Rebuild a node verbatim with its children recursively simplified.
    fn copy_node(&mut self, _reference: NodeRef, node: &RawNode) -> smt_wire::Result<NodeRef> {
        let width = node.width;
        match node.tag {
            tag::BV_VAR => {
                let name = self
                    .view
                    .blob_str(node.blob_ref(), "BV variable")?
                    .to_owned();
                self.builder.bv_var(&name, width)
            }
            tag::BV_CONST => {
                if width <= MAX_COBRA_WIDTH {
                    self.builder
                        .bv_const(node.payload & mask_for_width(width), width)
                } else {
                    let bytes = self.view.blob_ref(node.blob_ref())?.to_vec();
                    self.builder.bv_const_wide(&bytes, width)
                }
            }
            tag::BV_NOT => {
                let child = self.processed_child(node, 0)?;
                self.builder.bv_not(child)
            }
            tag::BV_NEG => {
                let child = self.processed_child(node, 0)?;
                self.builder.bv_neg(child)
            }
            tag::BV_AND => self.copy_nary(node, ExprBuilder::bv_and),
            tag::BV_OR => self.copy_nary(node, ExprBuilder::bv_or),
            tag::BV_XOR => self.copy_nary(node, ExprBuilder::bv_xor),
            tag::BV_ADD => self.copy_nary(node, ExprBuilder::bv_add),
            tag::BV_MUL => self.copy_nary(node, ExprBuilder::bv_mul),
            tag::BV_SUB => self.copy_binary(node, ExprBuilder::bv_sub),
            tag::BV_UDIV => self.copy_binary(node, ExprBuilder::bv_udiv),
            tag::BV_UREM => self.copy_binary(node, ExprBuilder::bv_urem),
            tag::BV_SDIV => self.copy_binary(node, ExprBuilder::bv_sdiv),
            tag::BV_SREM => self.copy_binary(node, ExprBuilder::bv_srem),
            tag::BV_SMOD => self.copy_binary(node, ExprBuilder::bv_smod),
            tag::BV_SHL => self.copy_binary(node, ExprBuilder::bv_shl),
            tag::BV_LSHR => self.copy_binary(node, ExprBuilder::bv_lshr),
            tag::BV_ASHR => self.copy_binary(node, ExprBuilder::bv_ashr),
            tag::BV_EXTRACT => {
                let child = self.processed_child(node, 0)?;
                self.builder
                    .bv_extract(child, node.aux_hi as u32, node.aux_lo)
            }
            tag::BV_CONCAT => {
                let high = self.processed_child(node, 0)?;
                let low = self.processed_child(node, 1)?;
                self.builder.bv_concat(high, low)
            }
            tag::BV_ZEXT => {
                let child = self.processed_child(node, 0)?;
                self.builder.bv_zext(child, node.aux_hi)
            }
            tag::BV_SEXT => {
                let child = self.processed_child(node, 0)?;
                self.builder.bv_sext(child, node.aux_hi)
            }
            tag::BV_ITE => {
                let cond = self.processed_child(node, 0)?;
                let then_value = self.processed_child(node, 1)?;
                let else_value = self.processed_child(node, 2)?;
                self.builder.bv_ite(cond, then_value, else_value)
            }
            tag::BV_SELECT => self.copy_select(node),
            tag::BOOL_TRUE => self.builder.bool_true(),
            tag::BOOL_FALSE => self.builder.bool_false(),
            tag::BOOL_VAR => {
                let name = self
                    .view
                    .blob_str(node.blob_ref(), "Bool variable")?
                    .to_owned();
                self.builder.bool_var(&name)
            }
            tag::BOOL_NOT => {
                let child = self.processed_child(node, 0)?;
                self.builder.bool_not(child)
            }
            tag::BOOL_AND => self.copy_binary(node, ExprBuilder::bool_and),
            tag::BOOL_OR => self.copy_binary(node, ExprBuilder::bool_or),
            tag::BOOL_IMPLIES => self.copy_binary(node, ExprBuilder::bool_implies),
            tag::BV_EQ => self.copy_binary(node, ExprBuilder::bv_eq),
            tag::BV_ULT => self.copy_binary(node, ExprBuilder::bv_ult),
            tag::BV_ULE => self.copy_binary(node, ExprBuilder::bv_ule),
            tag::BV_SLT => self.copy_binary(node, ExprBuilder::bv_slt),
            tag::BV_SLE => self.copy_binary(node, ExprBuilder::bv_sle),
            tag::UADD_OVF => self.copy_binary(node, ExprBuilder::uadd_ovf),
            tag::SADD_OVF => self.copy_binary(node, ExprBuilder::sadd_ovf),
            tag::USUB_OVF => self.copy_binary(node, ExprBuilder::usub_ovf),
            tag::SSUB_OVF => self.copy_binary(node, ExprBuilder::ssub_ovf),
            tag::UMUL_OVF => self.copy_binary(node, ExprBuilder::umul_ovf),
            tag::SMUL_OVF => self.copy_binary(node, ExprBuilder::smul_ovf),
            tag::NEG_OVF => {
                let child = self.processed_child(node, 0)?;
                self.builder.neg_ovf(child)
            }
            tag::SDIV_OVF => self.copy_binary(node, ExprBuilder::sdiv_ovf),
            other => Err(WireError::invalid(
                "simplify",
                format!("unsupported node tag {other}"),
            )),
        }
    }

    fn copy_nary(
        &mut self,
        node: &RawNode,
        op: fn(&mut ExprBuilder, NodeRef, NodeRef) -> smt_wire::Result<NodeRef>,
    ) -> smt_wire::Result<NodeRef> {
        let arity = node.arity as u32;
        if arity == 0 {
            return Err(WireError::invalid(
                "simplify",
                "n-ary node with no children",
            ));
        }
        let mut acc = self.processed_child(node, 0)?;
        for offset in 1..arity {
            let child = self.processed_child(node, offset)?;
            acc = op(&mut self.builder, acc, child)?;
        }
        Ok(acc)
    }

    fn copy_binary(
        &mut self,
        node: &RawNode,
        op: fn(&mut ExprBuilder, NodeRef, NodeRef) -> smt_wire::Result<NodeRef>,
    ) -> smt_wire::Result<NodeRef> {
        let a = self.processed_child(node, 0)?;
        let b = self.processed_child(node, 1)?;
        op(&mut self.builder, a, b)
    }

    fn copy_select(&mut self, node: &RawNode) -> smt_wire::Result<NodeRef> {
        let pairs = node.aux_hi as u32;
        let mut selectors = Vec::with_capacity(pairs as usize);
        let mut values = Vec::with_capacity(pairs as usize);
        for pair in 0..pairs {
            selectors.push(self.processed_child(node, 2 * pair)?);
            values.push(self.processed_child(node, 2 * pair + 1)?);
        }
        let default = self.processed_child(node, 2 * pairs)?;
        self.builder.bv_select(&selectors, &values, default)
    }
}

/// True when `expr` only uses operators and variable indices this backend can
/// lower back into a uniform-width wire island.
fn is_lowerable(expr: &CobraExpr, var_count: usize) -> bool {
    match expr.kind {
        CobraKind::Variable(index) => (index as usize) < var_count,
        CobraKind::Constant(_) => true,
        CobraKind::Add
        | CobraKind::Mul
        | CobraKind::And
        | CobraKind::Or
        | CobraKind::Xor
        | CobraKind::Not
        | CobraKind::Neg
        | CobraKind::Shr(_) => expr
            .children
            .iter()
            .all(|child| is_lowerable(child, var_count)),
        CobraKind::ZExt(_) | CobraKind::SExt(_) | CobraKind::Trunc(_) | CobraKind::Concat => false,
    }
}

/// A leaf of a CoBRA MBA island: either a real bit-vector variable, or an
/// opaque boundary standing in for a non-MBA / wrong-width subtree that will be
/// simplified recursively and spliced back when the island is lowered.
enum IslandVar {
    Named(String),
    Boundary(NodeRef),
}

/// CoBRA identifies variables by index but requires unique names alongside.
/// Real variables keep their wire names; boundaries get synthetic names,
/// disambiguated against everything else with underscore suffixes.
fn island_var_names(vars: &[IslandVar]) -> Vec<String> {
    let mut used: std::collections::HashSet<String> = vars
        .iter()
        .filter_map(|var| match var {
            IslandVar::Named(name) => Some(name.clone()),
            IslandVar::Boundary(_) => None,
        })
        .collect();
    vars.iter()
        .enumerate()
        .map(|(index, var)| match var {
            IslandVar::Named(name) => name.clone(),
            IslandVar::Boundary(_) => {
                let mut name = format!("__cobra_boundary{index}");
                while !used.insert(name.clone()) {
                    name.push('_');
                }
                name
            }
        })
        .collect()
}

struct IslandConversion<'a, 'b> {
    view: ExprView<'a>,
    width: u32,
    vars: Vec<IslandVar>,
    by_name: HashMap<String, usize>,
    by_ref: HashMap<NodeRef, usize>,
    /// Node-visit budget shared across every island attempt of one request;
    /// see [`ISLAND_BUDGET_FACTOR`].
    budget: &'b mut usize,
}

impl<'a, 'b> IslandConversion<'a, 'b> {
    fn new(view: ExprView<'a>, width: u32, budget: &'b mut usize) -> Self {
        Self {
            view,
            width,
            vars: Vec::new(),
            by_name: HashMap::new(),
            by_ref: HashMap::new(),
            budget,
        }
    }

    /// Convert a subtree to a CoBRA expression at the island's width. MBA
    /// operators of the island width recurse; anything else (other ops, a
    /// different width, a wide constant, a Bool) becomes an opaque boundary
    /// variable.
    ///
    /// Conversion never abandons an island for being too large: CoBRA enforces
    /// its own variable and node budgets in `simplify_expr` and the resulting
    /// error makes the caller copy the island structurally.
    fn convert(&mut self, reference: NodeRef) -> smt_wire::Result<Arc<CobraExpr>> {
        *self.budget = self.budget.checked_sub(1).ok_or_else(|| {
            WireError::invalid("simplify", "cobra island conversion budget exhausted")
        })?;
        if !reference.is_bv() {
            return Ok(self.boundary(reference));
        }
        let node = self.view.node(reference.index())?;
        if node.width != self.width {
            return Ok(self.boundary(reference));
        }
        Ok(match node.tag {
            tag::BV_VAR => self.named(reference, &node)?,
            tag::BV_CONST if node.width <= MAX_COBRA_WIDTH => {
                CobraExpr::constant(node.payload & mask_for_width(self.width))
            }
            tag::BV_NOT => CobraExpr::not(self.convert_child(&node, 0)?),
            tag::BV_NEG => CobraExpr::neg(self.convert_child(&node, 0)?),
            tag::BV_AND => self.fold_nary(&node, CobraExpr::and)?,
            tag::BV_OR => self.fold_nary(&node, CobraExpr::or)?,
            tag::BV_XOR => self.fold_nary(&node, CobraExpr::xor)?,
            tag::BV_ADD => self.fold_nary(&node, CobraExpr::add)?,
            tag::BV_MUL => self.fold_nary(&node, CobraExpr::mul)?,
            tag::BV_SUB => {
                let a = self.convert_child(&node, 0)?;
                let b = self.convert_child(&node, 1)?;
                CobraExpr::add(a, CobraExpr::neg(b))
            }
            // A logical shift by a constant amount stays inside the island;
            // CoBRA pins the amount into the node. Shifts by an expression
            // become boundaries.
            tag::BV_LSHR => match self.constant_shift_amount(&node)? {
                Some(amount) => CobraExpr::shr(self.convert_child(&node, 0)?, amount),
                None => self.boundary(reference),
            },
            _ => self.boundary(reference),
        })
    }

    /// The shift amount when child 1 is a narrow constant, `None` otherwise.
    fn constant_shift_amount(&self, node: &RawNode) -> smt_wire::Result<Option<u64>> {
        let amount_ref = self.src_child(node, 1)?;
        if !amount_ref.is_bv() {
            return Ok(None);
        }
        let amount = self.view.node(amount_ref.index())?;
        if amount.tag != tag::BV_CONST || amount.width > MAX_COBRA_WIDTH {
            return Ok(None);
        }
        Ok(Some(amount.payload & mask_for_width(amount.width)))
    }

    fn convert_child(&mut self, node: &RawNode, offset: u32) -> smt_wire::Result<Arc<CobraExpr>> {
        let child = self.src_child(node, offset)?;
        self.convert(child)
    }

    /// Fold a wire n-ary node into CoBRA's binary tree shape.
    fn fold_nary(
        &mut self,
        node: &RawNode,
        op: fn(Arc<CobraExpr>, Arc<CobraExpr>) -> Arc<CobraExpr>,
    ) -> smt_wire::Result<Arc<CobraExpr>> {
        let arity = node.arity as u32;
        if arity == 0 {
            return Err(WireError::invalid(
                "simplify",
                "n-ary node with no children",
            ));
        }
        let mut acc = self.convert_child(node, 0)?;
        for offset in 1..arity {
            let child = self.convert_child(node, offset)?;
            acc = op(acc, child);
        }
        Ok(acc)
    }

    fn named(&mut self, reference: NodeRef, node: &RawNode) -> smt_wire::Result<Arc<CobraExpr>> {
        if let Some(&id) = self.by_ref.get(&reference) {
            return Ok(CobraExpr::variable(id as u32));
        }
        let name = self
            .view
            .blob_str(node.blob_ref(), "BV variable")?
            .to_owned();
        if let Some(&id) = self.by_name.get(&name) {
            self.by_ref.insert(reference, id);
            return Ok(CobraExpr::variable(id as u32));
        }
        let id = self.vars.len();
        self.vars.push(IslandVar::Named(name.clone()));
        self.by_ref.insert(reference, id);
        self.by_name.insert(name, id);
        Ok(CobraExpr::variable(id as u32))
    }

    fn boundary(&mut self, reference: NodeRef) -> Arc<CobraExpr> {
        if let Some(&id) = self.by_ref.get(&reference) {
            return CobraExpr::variable(id as u32);
        }
        let id = self.vars.len();
        self.vars.push(IslandVar::Boundary(reference));
        self.by_ref.insert(reference, id);
        CobraExpr::variable(id as u32)
    }

    fn src_child(&self, node: &RawNode, offset: u32) -> smt_wire::Result<NodeRef> {
        let index = node
            .children
            .checked_add(offset)
            .ok_or(WireError::IntegerOverflow("child array index"))?;
        self.view.child_ref(index)
    }
}

fn mask_for_width(width: u32) -> u64 {
    debug_assert!((1..=MAX_COBRA_WIDTH).contains(&width));
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}
