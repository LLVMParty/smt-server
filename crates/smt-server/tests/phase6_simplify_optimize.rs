use smt_server::{
    handle_binary_frame, Backend, BinbitBackend, QfbvsmtrsBackend, QueryResult, Z3Backend,
};
use smt_wire::raw::{
    response_flags, BinaryRequest, BinaryResponse, ExprBuilder, ModelBlock, ModelEntry,
    OptimizationValueBlock, ScalarValue, SimplifyBlock, Status, UnsatCoreBlock,
};

#[test]
fn simplify_returns_simplify_block() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 8).unwrap();
    let one = builder.bv_const(1, 8).unwrap();
    let target = builder.bv_add(x, one).unwrap();
    let request = builder.build_simplify_request(21, target).unwrap();
    let response = handle_binary_frame(&request, &BinbitBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Simplified);
    assert_eq!(response.envelope.flags, response_flags::HAS_EXPR);
    let block = SimplifyBlock::decode(&response.payload).unwrap();
    assert!(block.target_node.is_bv());
    let expr = block.expression_buffer().unwrap();
    assert_eq!(expr.view().unwrap().node_count(), 3);
}

#[test]
fn unsigned_minimize_and_maximize_return_optimum_values() {
    let mut min_builder = ExprBuilder::new();
    let x = min_builder.bv_var("x", 4).unwrap();
    let three = min_builder.bv_const(3, 4).unwrap();
    let ge = min_builder.bv_uge(x, three).unwrap();
    min_builder.assert(ge).unwrap();
    let request = min_builder
        .build_minimize_request(22, x, false, 0, false)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &BinbitBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
    assert_eq!(response.envelope.flags, response_flags::HAS_VALUE);
    let optimum = OptimizationValueBlock::decode(&response.payload, false).unwrap();
    assert_eq!(optimum.optimum.width, 4);
    assert_eq!(optimum.optimum.bytes, vec![3]);

    let mut max_builder = ExprBuilder::new();
    let y = max_builder.bv_var("y", 4).unwrap();
    let ten = max_builder.bv_const(10, 4).unwrap();
    let le = max_builder.bv_ule(y, ten).unwrap();
    max_builder.assert(le).unwrap();
    let request = max_builder
        .build_maximize_request(23, y, false, 0, false)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &BinbitBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    let optimum = OptimizationValueBlock::decode(&response.payload, false).unwrap();
    assert_eq!(optimum.optimum.bytes, vec![10]);
}

#[test]
fn signed_optimization_uses_signed_ordering_and_can_return_model() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let request = builder
        .build_minimize_request(24, x, true, 0, true)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &BinbitBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        response.envelope.flags,
        response_flags::HAS_VALUE | response_flags::HAS_MODEL
    );
    let optimum = OptimizationValueBlock::decode(&response.payload, true).unwrap();
    // 4-bit signed minimum is -8, encoded as 0b1000.
    assert_eq!(optimum.optimum.bytes, vec![8]);
    assert!(optimum
        .model
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.value.bytes == vec![8]));

    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let request = builder
        .build_maximize_request(25, x, true, 0, false)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &BinbitBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    let optimum = OptimizationValueBlock::decode(&response.payload, false).unwrap();
    // 4-bit signed maximum is +7.
    assert_eq!(optimum.optimum.bytes, vec![7]);
}

#[test]
fn qfbvsmtrs_backend_optimization_uses_bit_hunt() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let five = builder.bv_const(5, 4).unwrap();
    let ge = builder.bv_uge(x, five).unwrap();
    builder.assert(ge).unwrap();
    let request = builder
        .build_minimize_request(27, x, false, 0, true)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &QfbvsmtrsBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
    assert_eq!(
        response.envelope.flags,
        response_flags::HAS_VALUE | response_flags::HAS_MODEL
    );
    let optimum = OptimizationValueBlock::decode(&response.payload, true).unwrap();
    assert_eq!(optimum.optimum.bytes, vec![5]);
    assert!(optimum
        .model
        .unwrap()
        .entries
        .iter()
        .any(|entry| entry.value.bytes == vec![5]));
}

#[test]
fn z3_backend_optimization_uses_bit_hunt() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let five = builder.bv_const(5, 4).unwrap();
    let ge = builder.bv_uge(x, five).unwrap();
    builder.assert(ge).unwrap();
    let request = builder
        .build_minimize_request(26, x, false, 0, false)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &Z3Backend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
    let optimum = OptimizationValueBlock::decode(&response.payload, false).unwrap();
    assert_eq!(optimum.optimum.bytes, vec![5]);
}

#[test]
fn bitwuzla_backend_optimization_uses_bit_hunt() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let five = builder.bv_const(5, 4).unwrap();
    let ge = builder.bv_uge(x, five).unwrap();
    builder.assert(ge).unwrap();
    let request = builder
        .build_minimize_request(28, x, false, 0, false)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &smt_server::BitwuzlaBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
    let optimum = OptimizationValueBlock::decode(&response.payload, false).unwrap();
    assert_eq!(optimum.optimum.bytes, vec![5]);
}

struct BadModelBackend;
impl Backend for BadModelBackend {
    fn name(&self) -> &'static str {
        "bad-model-test"
    }
    fn handle(&self, request: &BinaryRequest) -> smt_wire::Result<QueryResult> {
        Ok(QueryResult::sat(Some(ModelBlock {
            entries: vec![ModelEntry {
                node_ref: request.assertion_roots[0],
                value: ScalarValue::bool(true),
            }],
        })))
    }
}

struct BadCoreBackend;
impl Backend for BadCoreBackend {
    fn name(&self) -> &'static str {
        "bad-core-test"
    }
    fn handle(&self, _request: &BinaryRequest) -> smt_wire::Result<QueryResult> {
        Ok(QueryResult::unsat(Some(UnsatCoreBlock {
            names: vec!["not_from_request".to_owned()],
        })))
    }
}

struct BadOptimizationBackend;
impl Backend for BadOptimizationBackend {
    fn name(&self) -> &'static str {
        "bad-optimization-test"
    }
    fn handle(&self, _request: &BinaryRequest) -> smt_wire::Result<QueryResult> {
        Ok(QueryResult::sat_optimization(OptimizationValueBlock {
            optimum: ScalarValue {
                width: 8,
                bytes: vec![0],
            },
            model: None,
        }))
    }
}

#[test]
fn binary_response_rejects_model_entries_not_matching_request_variables() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 1).unwrap();
    let one = builder.bv_const(1, 1).unwrap();
    let eq = builder.bv_eq(x, one).unwrap();
    builder.assert(eq).unwrap();
    let request = builder.build_solve_request(31, 0, true, false).unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &BadModelBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Error);
    let message = String::from_utf8(response.payload).unwrap();
    assert!(message.contains("invalid backend model"), "{message}");
}

#[test]
fn binary_response_rejects_unsat_core_names_outside_request() {
    let mut builder = ExprBuilder::new();
    let p = builder.bool_var("p").unwrap();
    builder.assert_named("p_true", p).unwrap();
    let request = builder.build_solve_request(32, 0, false, true).unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &BadCoreBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Error);
    let message = String::from_utf8(response.payload).unwrap();
    assert!(message.contains("invalid backend unsat core"), "{message}");
}

#[test]
fn binary_response_rejects_optimization_value_with_wrong_width() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let request = builder
        .build_minimize_request(33, x, false, 0, false)
        .unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &BadOptimizationBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Error);
    let message = String::from_utf8(response.payload).unwrap();
    assert!(
        message.contains("invalid backend optimization block"),
        "{message}"
    );
}
