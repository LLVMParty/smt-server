//! CoBRA simplifies Lean-certified MBA islands, leaves everything else
//! unchanged, and composes with Rumba through the simplify chain.

use std::sync::Arc;

use smt_server::{handle_binary_frame, Backend, CobraBackend, RumbaBackend, SimplifyChainBackend};
use smt_wire::raw::{tag, BinaryResponse, ExprBuilder, NodeRef, SimplifyBlock, Status};

/// Count occurrences of each tag in the simplified result expression.
fn tag_counts(payload: &[u8]) -> Vec<u8> {
    let block = SimplifyBlock::decode(payload).unwrap();
    let buffer = block.expression_buffer().unwrap();
    let view = buffer.view().unwrap();
    let mut tags = Vec::new();
    for index in 0..view.node_count() {
        tags.push(view.node(index).unwrap().tag);
    }
    tags
}

fn simplify_with(backend: &dyn Backend, builder: &ExprBuilder, target: NodeRef) -> Vec<u8> {
    let request = builder.build_simplify_request(1, target).unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, backend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Simplified);
    response.payload
}

fn simplify(builder: &ExprBuilder, target: NodeRef) -> Vec<u8> {
    simplify_with(&CobraBackend::default(), builder, target)
}

/// `(x ^ y) + 2 * (x & y)` -> `x + y`, backed by CoBRA's Lean certificate.
#[test]
fn simplifies_certified_mba_identity() {
    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let y = b.bv_var("y", 64).unwrap();
    let two = b.bv_const(2, 64).unwrap();
    let xor = b.bv_xor(x, y).unwrap();
    let and = b.bv_and(x, y).unwrap();
    let mul = b.bv_mul(two, and).unwrap();
    let target = b.bv_add(xor, mul).unwrap();

    let tags = tag_counts(&simplify(&b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(count(tag::BV_XOR), 0, "xor not eliminated");
    assert_eq!(count(tag::BV_AND), 0, "and not eliminated");
    assert_eq!(count(tag::BV_MUL), 0, "mul not eliminated");
    assert!(count(tag::BV_ADD) >= 1, "reduced x+y add missing");
}

/// The same island simplifies at a narrow width; CoBRA replays its
/// certificates per bit-width.
#[test]
fn simplifies_mba_identity_at_narrow_width() {
    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 16).unwrap();
    let y = b.bv_var("y", 16).unwrap();
    let or = b.bv_or(x, y).unwrap();
    let and = b.bv_and(x, y).unwrap();
    let target = b.bv_add(or, and).unwrap();

    let tags = tag_counts(&simplify(&b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(count(tag::BV_OR), 0, "or not eliminated");
    assert_eq!(count(tag::BV_AND), 0, "and not eliminated");
    assert!(count(tag::BV_ADD) >= 1, "reduced x+y add missing");
}

/// `concat(0xFF, (x^y) + 2*(x&y))` -> the MBA arm must collapse while the
/// `concat` skeleton is preserved.
#[test]
fn simplifies_mba_island_nested_under_concat() {
    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let y = b.bv_var("y", 64).unwrap();
    let two = b.bv_const(2, 64).unwrap();
    let xor = b.bv_xor(x, y).unwrap();
    let and = b.bv_and(x, y).unwrap();
    let mul = b.bv_mul(two, and).unwrap();
    let mba = b.bv_add(xor, mul).unwrap();
    let hi = b.bv_const(0xFF, 16).unwrap();
    let target = b.bv_concat(hi, mba).unwrap();

    let tags = tag_counts(&simplify(&b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(count(tag::BV_XOR), 0, "xor not eliminated");
    assert_eq!(count(tag::BV_AND), 0, "and not eliminated");
    assert_eq!(count(tag::BV_MUL), 0, "mul not eliminated");
    assert_eq!(count(tag::BV_CONCAT), 1, "concat skeleton not preserved");
}

/// A non-MBA subtree becomes an opaque boundary; the identity `boundary ^ 0`
/// island around it must still collapse without touching the `udiv` inside.
#[test]
fn preserves_non_mba_boundary_subtrees() {
    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let y = b.bv_var("y", 64).unwrap();
    let div = b.bv_udiv(x, y).unwrap();
    let or = b.bv_or(div, div).unwrap();
    let target = b.bv_or(or, div).unwrap();

    let tags = tag_counts(&simplify(&b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(count(tag::BV_UDIV), 1, "udiv boundary not preserved");
    assert_eq!(count(tag::BV_OR), 0, "or self-absorption not eliminated");
}

/// A wire variable that collides with the synthetic boundary-name prefix must
/// not break variable naming: the result is still a valid simplification.
#[test]
fn tolerates_boundary_name_collisions() {
    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let y = b.bv_var("y", 64).unwrap();
    let clash = b.bv_var("__cobra_boundary0", 64).unwrap();
    let div = b.bv_udiv(x, y).unwrap();
    let add = b.bv_add(div, clash).unwrap();
    let target = b.bv_xor(add, add).unwrap();

    let tags = tag_counts(&simplify(&b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(
        count(tag::BV_XOR),
        0,
        "xor self-cancellation not eliminated"
    );
}

/// Islands CoBRA cannot improve keep their structural skeleton intact.
#[test]
fn unimproved_islands_keep_their_skeleton() {
    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let mask = b.bv_const(0xFF, 64).unwrap();
    let and = b.bv_and(x, mask).unwrap();
    let shifted = b.bv_shl(and, mask).unwrap();
    let target = b.bv_add(shifted, and).unwrap();

    let tags = tag_counts(&simplify(&b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(count(tag::BV_SHL), 1, "shl skeleton not preserved");
    assert_eq!(count(tag::BV_ADD), 1, "add skeleton not preserved");
}

/// The chain feeds Rumba's output into CoBRA; each backend's specialty lands
/// in one pass through the chain.
#[test]
fn chain_composes_rumba_and_cobra() {
    let chain = SimplifyChainBackend::new(vec![
        Arc::new(RumbaBackend),
        Arc::new(CobraBackend::default()),
    ]);

    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let y = b.bv_var("y", 64).unwrap();
    let two = b.bv_const(2, 64).unwrap();
    let xor = b.bv_xor(x, y).unwrap();
    let and = b.bv_and(x, y).unwrap();
    let mul = b.bv_mul(two, and).unwrap();
    let target = b.bv_add(xor, mul).unwrap();

    let tags = tag_counts(&simplify_with(&chain, &b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(count(tag::BV_XOR), 0, "xor not eliminated");
    assert_eq!(count(tag::BV_AND), 0, "and not eliminated");
    assert_eq!(count(tag::BV_MUL), 0, "mul not eliminated");
    assert!(count(tag::BV_ADD) >= 1, "reduced x+y add missing");
}

/// A chain still produces a valid identity result when every stage declines.
#[test]
fn chain_falls_back_to_identity() {
    let chain = SimplifyChainBackend::new(vec![Arc::new(CobraBackend::default())]);

    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let y = b.bv_var("y", 64).unwrap();
    let target = b.bv_udiv(x, y).unwrap();

    let tags = tag_counts(&simplify_with(&chain, &b, target));
    let count = |t: u8| tags.iter().filter(|&&n| n == t).count();
    assert_eq!(count(tag::BV_UDIV), 1, "udiv not preserved");
}

/// A pre-cancelled context yields an inconclusive result, matching the
/// backend cancellation convention, instead of an identity simplification.
#[test]
fn chain_observes_pre_cancelled_context() {
    use smt_server::{CancellationToken, SolveContext};
    use smt_wire::raw::BinaryRequest;

    let chain = SimplifyChainBackend::new(vec![
        Arc::new(RumbaBackend),
        Arc::new(CobraBackend::default()),
    ]);

    let mut b = ExprBuilder::new();
    let x = b.bv_var("x", 64).unwrap();
    let y = b.bv_var("y", 64).unwrap();
    let target = b.bv_xor(x, y).unwrap();
    let request = BinaryRequest::parse(&b.build_simplify_request(1, target).unwrap()).unwrap();

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let context = SolveContext::new(cancellation);
    let result = chain.handle_with_context(&request, &context).unwrap();
    assert!(!result.is_conclusive(), "chain returned {result:?}");
    assert!(
        result
            .message
            .as_deref()
            .unwrap_or_default()
            .contains("cancelled"),
        "missing cancellation message: {result:?}"
    );
}
