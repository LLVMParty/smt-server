//! Backend-level tests for [`BitwuzlaBackend`] in isolation (no racing),
//! exercising translate, solve, model extraction, named-unsat-core, and
//! bit-hunt optimize. CI runs these on Linux, macOS, and Windows MSVC.

use smt_server::{Backend, BitwuzlaBackend, QueryStatus};
use smt_wire::raw::{BinaryRequest, ExprBuilder};

#[test]
fn bitwuzla_backend_solves_and_returns_model() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let two = builder.bv_const(2, 4).unwrap();
    let eq = builder.bv_eq(x, two).unwrap();
    builder.assert(eq).unwrap();
    let request =
        BinaryRequest::parse(&builder.build_solve_request(1, 0, true, false).unwrap()).unwrap();
    let result = BitwuzlaBackend.handle(&request).unwrap();
    assert_eq!(result.status, QueryStatus::Sat);
    let model = result.model.expect("sat result should carry a model");
    assert!(
        model
            .entries
            .iter()
            .any(|entry| entry.value.width == 4 && entry.value.bytes == vec![2]),
        "model should assign x = 2, got {model:?}"
    );
}

#[test]
fn bitwuzla_backend_extracts_named_unsat_core() {
    let mut builder = ExprBuilder::new();
    let p = builder.bool_var("p").unwrap();
    let not_p = builder.bool_not(p).unwrap();
    builder.assert_named("p_true", p).unwrap();
    builder.assert_named("p_false", not_p).unwrap();
    let request =
        BinaryRequest::parse(&builder.build_solve_request(1, 0, false, true).unwrap()).unwrap();
    let result = BitwuzlaBackend.handle(&request).unwrap();
    assert_eq!(result.status, QueryStatus::Unsat);
    let core = result.core.expect("unsat result should carry a core");
    assert!(core.names.contains(&"p_true".to_owned()));
    assert!(core.names.contains(&"p_false".to_owned()));
}

#[test]
fn bitwuzla_backend_optimization_uses_bit_hunt() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let five = builder.bv_const(5, 4).unwrap();
    let ge = builder.bv_uge(x, five).unwrap();
    builder.assert(ge).unwrap();
    let request = BinaryRequest::parse(
        &builder
            .build_minimize_request(1, x, false, 0, true)
            .unwrap(),
    )
    .unwrap();
    let result = BitwuzlaBackend.handle(&request).unwrap();
    assert_eq!(result.status, QueryStatus::Sat);
    let optimization = result
        .optimization
        .expect("minimize result should carry an optimum");
    assert_eq!(optimization.optimum.bytes, vec![5]);
    let model = optimization
        .model
        .expect("minimize result should carry a model");
    assert!(
        model
            .entries
            .iter()
            .any(|entry| entry.value.bytes == vec![5]),
        "model should assign x = 5, got {model:?}"
    );
}
