use std::collections::HashMap;

use rumba_core::{
    expr::{Expr as RumbaExpr, VarId},
    simplify::simplify_mba,
};
use smt_wire::raw::{
    tag, BinaryRequest, Command, ExprBuilder, ExprView, NodeRef, RawNode, SimplifyBlock, WireError,
};

use crate::backend::{Backend, QueryResult, SolveContext};

const MAX_RUMBA_WIDTH: u32 = 64;

#[derive(Debug, Clone, Default)]
pub struct RumbaBackend;

impl Backend for RumbaBackend {
    fn name(&self) -> &'static str {
        "rumba"
    }

    fn handle(&self, request: &BinaryRequest) -> smt_wire::Result<QueryResult> {
        match request.envelope.command {
            Command::Simplify => simplify_request(request).map(QueryResult::simplified),
            Command::Solve | Command::Minimize | Command::Maximize => {
                Ok(QueryResult::unknown("rumba only supports SIMPLIFY"))
            }
        }
    }

    fn handle_with_context(
        &self,
        request: &BinaryRequest,
        context: &SolveContext,
    ) -> smt_wire::Result<QueryResult> {
        if context.is_cancelled() {
            return Ok(QueryResult::unknown("rumba request cancelled before start"));
        }
        self.handle(request)
    }
}

/// Simplify the target expression by rewriting each maximal MBA island in place.
///
/// Rumba only simplifies a pure same-width bit-vector MBA tree. Real lifted
/// expressions interleave that MBA with structural operators (`ite`, `concat`,
/// `extract`, extends, comparisons, ...), so the previous all-or-nothing converter
/// bailed to identity whenever any such node appeared anywhere. This pass instead
/// walks the DAG, copies the structural skeleton verbatim, and hands every maximal
/// MBA island to Rumba -- abstracting any non-MBA / wrong-width child of an island as
/// a fresh opaque Rumba variable that maps back to the recursively-simplified subtree.
fn simplify_request(request: &BinaryRequest) -> smt_wire::Result<SimplifyBlock> {
    let target = request
        .target_ref()
        .ok_or_else(|| WireError::invalid("simplify request", "missing target_node"))?;
    let view = request.expression_view()?;
    let mut simplifier = IslandSimplifier::new(view);
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

/// True for the bit-vector operators Rumba understands; these are the roots of MBA
/// islands. Leaves (VAR/CONST) are copied directly; everything else is structural.
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

struct IslandSimplifier<'a> {
    view: ExprView<'a>,
    builder: ExprBuilder,
    memo: HashMap<NodeRef, NodeRef>,
}

impl<'a> IslandSimplifier<'a> {
    fn new(view: ExprView<'a>) -> Self {
        Self {
            view,
            builder: ExprBuilder::new(),
            memo: HashMap::new(),
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
        if reference.is_bv() && is_mba_op(node.tag) && (1..=MAX_RUMBA_WIDTH).contains(&node.width) {
            if let Some(simplified) = self.try_island(reference, &node)? {
                return Ok(simplified);
            }
        }
        self.copy_node(reference, &node)
    }

    /// Convert the MBA island rooted at `reference`, simplify it with Rumba, and lower
    /// the result back into the builder. Returns `None` (so the caller copies the node
    /// structurally instead) if Rumba declines the island.
    fn try_island(
        &mut self,
        reference: NodeRef,
        node: &RawNode,
    ) -> smt_wire::Result<Option<NodeRef>> {
        let width = node.width;
        let bits = width as u8;
        let mut conversion = IslandConversion::new(self.view, width);
        let expr = conversion.convert(reference)?;
        let vars = conversion.vars;
        // A `SolveError` means Rumba could not handle this island, not that the request
        // is bad: leave the island alone and let the caller copy it structurally.
        let simplified = match simplify_mba(expr, bits) {
            Ok(simplified) => simplified,
            Err(_) => return Ok(None),
        };
        let root = self.lower(&simplified, width, &vars)?;
        Ok(Some(root))
    }

    /// Lower a simplified Rumba expression into the builder; opaque boundary variables
    /// resolve to the recursively-simplified subtrees they abstracted.
    fn lower(
        &mut self,
        expr: &RumbaExpr,
        width: u32,
        vars: &[IslandVar],
    ) -> smt_wire::Result<NodeRef> {
        let mask = mask_for_width(width);
        match expr {
            RumbaExpr::Var(var) => match vars.get(var.0) {
                Some(IslandVar::Named(name)) => self.builder.bv_var(name, width),
                Some(IslandVar::Boundary(reference)) => self.process(*reference),
                None => Err(WireError::invalid(
                    "rumba expression",
                    format!("unknown Rumba variable v{}", var.0),
                )),
            },
            RumbaExpr::Const(value) => self.builder.bv_const(value & mask, width),
            RumbaExpr::Not(child) => {
                let child = self.lower(child, width, vars)?;
                self.builder.bv_not(child)
            }
            RumbaExpr::Scale(coeff, child) => {
                let child = self.lower(child, width, vars)?;
                let coeff = self.builder.bv_const(coeff & mask, width)?;
                self.builder.bv_mul(coeff, child)
            }
            RumbaExpr::And(children) => {
                self.lower_nary(children, width, vars, mask, ExprBuilder::bv_and)
            }
            RumbaExpr::Or(children) => {
                self.lower_nary(children, width, vars, 0, ExprBuilder::bv_or)
            }
            RumbaExpr::Xor(children) => {
                self.lower_nary(children, width, vars, 0, ExprBuilder::bv_xor)
            }
            RumbaExpr::Add(children) => {
                self.lower_nary(children, width, vars, 0, ExprBuilder::bv_add)
            }
            RumbaExpr::Mul(children) => {
                self.lower_nary(children, width, vars, 1, ExprBuilder::bv_mul)
            }
        }
    }

    fn lower_nary(
        &mut self,
        children: &[RumbaExpr],
        width: u32,
        vars: &[IslandVar],
        identity: u64,
        op: fn(&mut ExprBuilder, NodeRef, NodeRef) -> smt_wire::Result<NodeRef>,
    ) -> smt_wire::Result<NodeRef> {
        if children.is_empty() {
            return self.builder.bv_const(identity, width);
        }
        let mut refs = Vec::with_capacity(children.len());
        for child in children {
            refs.push(self.lower(child, width, vars)?);
        }
        let mut acc = refs[0];
        for &child in &refs[1..] {
            acc = op(&mut self.builder, acc, child)?;
        }
        Ok(acc)
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
                if width <= MAX_RUMBA_WIDTH {
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

/// A leaf of a Rumba MBA island: either a real bit-vector variable, or an opaque
/// boundary standing in for a non-MBA / wrong-width subtree that will be simplified
/// recursively and spliced back when the island is lowered.
enum IslandVar {
    Named(String),
    Boundary(NodeRef),
}

struct IslandConversion<'a> {
    view: ExprView<'a>,
    width: u32,
    vars: Vec<IslandVar>,
    by_name: HashMap<String, usize>,
    by_ref: HashMap<NodeRef, usize>,
}

impl<'a> IslandConversion<'a> {
    fn new(view: ExprView<'a>, width: u32) -> Self {
        Self {
            view,
            width,
            vars: Vec::new(),
            by_name: HashMap::new(),
            by_ref: HashMap::new(),
        }
    }

    /// Convert a subtree to a Rumba expression at the island's width. MBA operators of
    /// the island width recurse; anything else (other ops, a different width, a wide
    /// constant, a Bool) becomes an opaque boundary variable.
    ///
    /// Conversion never abandons an island for being too large: Rumba applies its own
    /// variable limit to each linear sub-MBA after reduction, and reports it as
    /// `SolveError::TooManyVariables`. A cap applied here would count island leaves
    /// before reduction, which is a different (and not conservative) number.
    fn convert(&mut self, reference: NodeRef) -> smt_wire::Result<RumbaExpr> {
        if !reference.is_bv() {
            return Ok(self.boundary(reference));
        }
        let node = self.view.node(reference.index())?;
        if node.width != self.width {
            return Ok(self.boundary(reference));
        }
        Ok(match node.tag {
            tag::BV_VAR => self.named(reference, &node)?,
            tag::BV_CONST if node.width <= MAX_RUMBA_WIDTH => {
                RumbaExpr::Const(node.payload & mask_for_width(self.width))
            }
            tag::BV_NOT => !self.convert_child(&node, 0)?,
            tag::BV_NEG => -self.convert_child(&node, 0)?,
            tag::BV_AND => RumbaExpr::And(self.nary(reference, tag::BV_AND)?),
            tag::BV_OR => RumbaExpr::Or(self.nary(reference, tag::BV_OR)?),
            tag::BV_XOR => RumbaExpr::Xor(self.nary(reference, tag::BV_XOR)?),
            tag::BV_ADD => RumbaExpr::Add(self.nary(reference, tag::BV_ADD)?),
            tag::BV_MUL => RumbaExpr::Mul(self.nary(reference, tag::BV_MUL)?),
            tag::BV_SUB => {
                let (a, b) = self.children2(&node)?;
                a - b
            }
            _ => self.boundary(reference),
        })
    }

    fn convert_child(&mut self, node: &RawNode, offset: u32) -> smt_wire::Result<RumbaExpr> {
        let child = self.src_child(node, offset)?;
        self.convert(child)
    }

    fn children2(&mut self, node: &RawNode) -> smt_wire::Result<(RumbaExpr, RumbaExpr)> {
        let a = self.convert_child(node, 0)?;
        let b = self.convert_child(node, 1)?;
        Ok((a, b))
    }

    fn nary(&mut self, root: NodeRef, tag: u8) -> smt_wire::Result<Vec<RumbaExpr>> {
        let mut stack = vec![root];
        let mut terms = Vec::new();
        while let Some(reference) = stack.pop() {
            let node = self.view.node(reference.index())?;
            if reference.is_bv() && node.tag == tag && node.width == self.width {
                for offset in (0..node.arity as u32).rev() {
                    stack.push(self.src_child(&node, offset)?);
                }
            } else {
                terms.push(self.convert(reference)?);
            }
        }
        Ok(terms)
    }

    fn named(&mut self, reference: NodeRef, node: &RawNode) -> smt_wire::Result<RumbaExpr> {
        if let Some(&id) = self.by_ref.get(&reference) {
            return Ok(RumbaExpr::Var(VarId(id)));
        }
        let name = self
            .view
            .blob_str(node.blob_ref(), "BV variable")?
            .to_owned();
        if let Some(&id) = self.by_name.get(&name) {
            self.by_ref.insert(reference, id);
            return Ok(RumbaExpr::Var(VarId(id)));
        }
        let id = self.vars.len();
        self.vars.push(IslandVar::Named(name.clone()));
        self.by_ref.insert(reference, id);
        self.by_name.insert(name, id);
        Ok(RumbaExpr::Var(VarId(id)))
    }

    fn boundary(&mut self, reference: NodeRef) -> RumbaExpr {
        if let Some(&id) = self.by_ref.get(&reference) {
            return RumbaExpr::Var(VarId(id));
        }
        let id = self.vars.len();
        self.vars.push(IslandVar::Boundary(reference));
        self.by_ref.insert(reference, id);
        RumbaExpr::Var(VarId(id))
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
    debug_assert!((1..=MAX_RUMBA_WIDTH).contains(&width));
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}
