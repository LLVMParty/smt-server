use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use smt_server::{
    handle_binary_frame, serve_tcp, Backend, BinbitBackend, QfbvsmtrsBackend, QueryResult,
    ServerConfig, Z3Backend,
};
use smt_wire::raw::{
    le, response_flags, status, tag, BinaryRequest, BinaryResponse, BlobRef, Command, ExprBuilder,
    ExpressionBuffer, ModelBlock, NodeRef, RawNode, Status, UnsatCoreBlock,
};

fn request_with_duplicate_bv_symbol_not_equal(request_id: u32) -> Vec<u8> {
    let x = BlobRef::new(0, 1).to_payload();
    let expression = ExpressionBuffer::from_parts(
        &[
            RawNode::new(tag::BV_VAR, 0, 0, 4, 0, 0, x),
            RawNode::new(tag::BV_VAR, 0, 0, 4, 0, 0, x),
            RawNode::new(tag::BV_EQ, 2, 0, 0, 0, 0, 0),
            RawNode::new(tag::BOOL_NOT, 1, 0, 0, 0, 2, 0),
        ],
        &[
            NodeRef::bv(0).unwrap(),
            NodeRef::bv(1).unwrap(),
            NodeRef::bool(2).unwrap(),
        ],
        b"x",
    )
    .unwrap()
    .into_bytes();
    BinaryRequest::new(
        request_id,
        Command::Solve,
        0,
        0,
        expression,
        vec![NodeRef::bool(3).unwrap()],
        vec![],
        vec![],
        None,
    )
    .unwrap()
    .encode()
    .unwrap()
}

fn request_with_duplicate_bv_symbol_model(request_id: u32) -> Vec<u8> {
    let x = BlobRef::new(0, 1).to_payload();
    let expression = ExpressionBuffer::from_parts(
        &[
            RawNode::new(tag::BV_VAR, 0, 0, 4, 0, 0, x),
            RawNode::new(tag::BV_VAR, 0, 0, 4, 0, 0, x),
            RawNode::new(tag::BV_CONST, 0, 0, 4, 0, 0, 3),
            RawNode::new(tag::BV_EQ, 2, 0, 0, 0, 0, 0),
        ],
        &[NodeRef::bv(0).unwrap(), NodeRef::bv(2).unwrap()],
        b"x",
    )
    .unwrap()
    .into_bytes();
    BinaryRequest::new(
        request_id,
        Command::Solve,
        smt_wire::raw::request_flags::WANT_MODEL,
        0,
        expression,
        vec![NodeRef::bool(3).unwrap()],
        vec![],
        vec![],
        None,
    )
    .unwrap()
    .encode()
    .unwrap()
}

#[test]
fn solve_backends_intern_duplicate_wire_bv_symbols_by_name() {
    let binbit = BinbitBackend;
    let z3 = Z3Backend;
    let qfbvsmtrs = QfbvsmtrsBackend;
    let bitwuzla = smt_server::BitwuzlaBackend;
    let backends: [(&str, &dyn Backend); 4] = [
        ("binbit", &binbit as &dyn Backend),
        ("z3", &z3 as &dyn Backend),
        ("qfbvsmtrs", &qfbvsmtrs as &dyn Backend),
        ("bitwuzla", &bitwuzla as &dyn Backend),
    ];
    for (name, backend) in backends {
        let request = request_with_duplicate_bv_symbol_not_equal(0x4455_5000);
        let response = handle_binary_frame(&request, backend)
            .unwrap()
            .encode()
            .unwrap();
        let response = BinaryResponse::parse(&response).unwrap();
        assert_eq!(response.envelope.status, Status::Unsat, "{name}");
    }
}

#[test]
fn solve_backends_return_models_for_duplicate_wire_bv_symbols() {
    let binbit = BinbitBackend;
    let z3 = Z3Backend;
    let qfbvsmtrs = QfbvsmtrsBackend;
    let bitwuzla = smt_server::BitwuzlaBackend;
    let backends: [(&str, &dyn Backend); 4] = [
        ("binbit", &binbit as &dyn Backend),
        ("z3", &z3 as &dyn Backend),
        ("qfbvsmtrs", &qfbvsmtrs as &dyn Backend),
        ("bitwuzla", &bitwuzla as &dyn Backend),
    ];
    for (name, backend) in backends {
        let request = request_with_duplicate_bv_symbol_model(0x4455_5001);
        let response = handle_binary_frame(&request, backend)
            .unwrap()
            .encode()
            .unwrap();
        let response = BinaryResponse::parse(&response).unwrap();
        assert_eq!(response.envelope.status, Status::Sat, "{name}");
        let model = ModelBlock::decode(&response.payload).unwrap();
        let first = model
            .entries
            .iter()
            .find(|entry| entry.node_ref == NodeRef::bv(0).unwrap())
            .unwrap_or_else(|| panic!("{name}: missing first x"));
        let second = model
            .entries
            .iter()
            .find(|entry| entry.node_ref == NodeRef::bv(1).unwrap())
            .unwrap_or_else(|| panic!("{name}: missing second x"));
        assert_eq!(first.value.bytes, vec![3], "{name}");
        assert_eq!(second.value.bytes, vec![3], "{name}");
    }
}

#[test]
fn binbit_backend_solves_sat_with_model() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 2).unwrap();
    let one = builder.bv_const(1, 2).unwrap();
    let eq = builder.bv_eq(x, one).unwrap();
    builder.assert(eq).unwrap();
    let request = builder.build_solve_request(11, 0, true, false).unwrap();

    let response = handle_binary_frame(&request, &BinbitBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
    assert_eq!(response.envelope.flags, response_flags::HAS_MODEL);
    let model = ModelBlock::decode(&response.payload).unwrap();
    let x_entry = model
        .entries
        .iter()
        .find(|entry| entry.node_ref == NodeRef::bv(0).unwrap())
        .unwrap();
    assert_eq!(x_entry.value.width, 2);
    assert_eq!(x_entry.value.bytes, vec![1]);
}

#[test]
fn binbit_backend_ignores_bv_const_unused_high_bits_by_v1_policy() {
    let expression = ExpressionBuffer::from_parts(
        &[
            RawNode::new(tag::BV_CONST, 0, 0, 4, 0, 0, 0xf1),
            RawNode::new(tag::BV_CONST, 0, 0, 4, 0, 0, 1),
            RawNode::new(tag::BV_EQ, 2, 0, 0, 0, 0, 0),
        ],
        &[NodeRef::bv(0).unwrap(), NodeRef::bv(1).unwrap()],
        &[],
    )
    .unwrap()
    .into_bytes();
    let request = BinaryRequest::new(
        44,
        Command::Solve,
        0,
        0,
        expression,
        vec![NodeRef::bool(2).unwrap()],
        vec![],
        vec![],
        None,
    )
    .unwrap()
    .encode()
    .unwrap();
    let response = handle_binary_frame(&request, &BinbitBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
}

#[test]
fn binbit_backend_solves_unsat_with_named_core() {
    let mut builder = ExprBuilder::new();
    let p = builder.bool_var("p").unwrap();
    let not_p = builder.bool_not(p).unwrap();
    builder.assert_named("p-is-true", p).unwrap();
    builder.assert_named("p-is-false", not_p).unwrap();
    let request = builder.build_solve_request(12, 0, false, true).unwrap();

    let response = handle_binary_frame(&request, &BinbitBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Unsat);
    assert_eq!(response.envelope.flags, response_flags::HAS_CORE);
    let core = UnsatCoreBlock::decode(&response.payload).unwrap();
    assert_eq!(core.names, vec!["p-is-true", "p-is-false"]);
}

#[test]
fn z3_backend_solves_sat_with_model() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let three = builder.bv_const(3, 4).unwrap();
    let eq = builder.bv_eq(x, three).unwrap();
    builder.assert(eq).unwrap();
    let request = builder.build_solve_request(13, 0, true, false).unwrap();

    let response = handle_binary_frame(&request, &Z3Backend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
    let model = ModelBlock::decode(&response.payload).unwrap();
    let x_entry = model
        .entries
        .iter()
        .find(|entry| entry.node_ref == NodeRef::bv(0).unwrap())
        .unwrap();
    assert_eq!(x_entry.value.width, 4);
    assert_eq!(x_entry.value.bytes, vec![3]);
}

#[test]
fn qfbvsmtrs_backend_solves_sat_with_model() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let one = builder.bv_const(1, 4).unwrap();
    let sum = builder.bv_add(x, one).unwrap();
    let three = builder.bv_const(3, 4).unwrap();
    let eq = builder.bv_eq(sum, three).unwrap();
    builder.assert(eq).unwrap();
    let request = builder.build_solve_request(15, 0, true, false).unwrap();

    let response = handle_binary_frame(&request, &QfbvsmtrsBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
    let model = ModelBlock::decode(&response.payload).unwrap();
    let x_entry = model
        .entries
        .iter()
        .find(|entry| entry.node_ref == NodeRef::bv(0).unwrap())
        .unwrap();
    assert_eq!(x_entry.value.width, 4);
    assert_eq!(x_entry.value.bytes, vec![2]);
}

#[test]
fn qfbvsmtrs_backend_solves_budgeted_request_with_bounded_backend() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let one = builder.bv_const(1, 4).unwrap();
    let eq = builder.bv_eq(x, one).unwrap();
    builder.assert(eq).unwrap();
    let request = builder.build_solve_request(18, 1000, false, false).unwrap();

    let response = handle_binary_frame(&request, &QfbvsmtrsBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Sat);
}

#[test]
fn qfbvsmtrs_backend_solves_unsat_with_named_core() {
    let mut builder = ExprBuilder::new();
    let p = builder.bool_var("p").unwrap();
    let not_p = builder.bool_not(p).unwrap();
    builder.assert_named("p-is-true", p).unwrap();
    builder.assert_named("p-is-false", not_p).unwrap();
    let request = builder.build_solve_request(17, 0, false, true).unwrap();

    let response = handle_binary_frame(&request, &QfbvsmtrsBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Unsat);
    assert_eq!(response.envelope.flags, response_flags::HAS_CORE);
    let core = UnsatCoreBlock::decode(&response.payload).unwrap();
    assert_eq!(core.names, vec!["p-is-true", "p-is-false"]);
}

#[test]
fn qfbvsmtrs_backend_solves_unsat() {
    let mut builder = ExprBuilder::new();
    let x = builder.bv_var("x", 4).unwrap();
    let one = builder.bv_const(1, 4).unwrap();
    let two = builder.bv_const(2, 4).unwrap();
    let eq_one = builder.bv_eq(x, one).unwrap();
    let eq_two = builder.bv_eq(x, two).unwrap();
    builder.assert(eq_one).unwrap();
    builder.assert(eq_two).unwrap();
    let request = builder.build_solve_request(16, 0, false, false).unwrap();

    let response = handle_binary_frame(&request, &QfbvsmtrsBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Unsat);
}

struct MissingModelBackend;

impl Backend for MissingModelBackend {
    fn name(&self) -> &'static str {
        "missing-model-test"
    }

    fn handle(&self, _request: &smt_wire::raw::BinaryRequest) -> smt_wire::Result<QueryResult> {
        Ok(QueryResult::sat(None))
    }
}

struct UnknownMessageBackend;

impl Backend for UnknownMessageBackend {
    fn name(&self) -> &'static str {
        "unknown-message-test"
    }

    fn handle(&self, _request: &smt_wire::raw::BinaryRequest) -> smt_wire::Result<QueryResult> {
        Ok(QueryResult::unknown("deadline elapsed"))
    }
}

#[test]
fn protocol_preserves_unknown_message() {
    let mut builder = ExprBuilder::new();
    let t = builder.bool_true().unwrap();
    builder.assert(t).unwrap();
    let request = builder.build_solve_request(98, 0, false, false).unwrap();

    let response = handle_binary_frame(&request, &UnknownMessageBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Unknown);
    assert_eq!(response.envelope.flags, response_flags::HAS_MESSAGE);
    assert_eq!(
        std::str::from_utf8(&response.payload).unwrap(),
        "deadline elapsed"
    );
}

#[test]
fn protocol_rejects_sat_without_requested_model() {
    let mut builder = ExprBuilder::new();
    let t = builder.bool_true().unwrap();
    builder.assert(t).unwrap();
    let request = builder.build_solve_request(99, 0, true, false).unwrap();

    let response = handle_binary_frame(&request, &MissingModelBackend)
        .unwrap()
        .encode()
        .unwrap();
    let response = BinaryResponse::parse(&response).unwrap();
    assert_eq!(response.envelope.status, Status::Error);
}

#[test]
fn tcp_server_handles_one_binary_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let handle = thread::spawn(move || {
        // The server runs until the client closes; this test relies on process
        // teardown to clean up the background thread after one request.
        let _ = serve_tcp(addr, ServerConfig::new(Arc::new(BinbitBackend)));
    });

    let mut builder = ExprBuilder::new();
    let t = builder.bool_true().unwrap();
    builder.assert(t).unwrap();
    let request = builder.build_solve_request(14, 0, false, false).unwrap();
    let frame = le::encode_transport_frame(&request).unwrap();

    let mut stream = None;
    for _ in 0..100 {
        match std::net::TcpStream::connect(addr) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    let mut stream = stream.unwrap();
    stream.write_all(&frame).unwrap();
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).unwrap();
    let len = u32::from_le_bytes(len) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).unwrap();
    let response = BinaryResponse::parse(&payload).unwrap();
    assert_eq!(response.envelope.request_id, 14);
    assert_eq!(response.envelope.status as u8, status::SAT);
    drop(stream);
    drop(handle);
}

#[test]
fn tcp_server_rejects_oversized_frame_before_allocation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let handle = thread::spawn(move || {
        let mut config = ServerConfig::new(Arc::new(BinbitBackend));
        config.max_frame_bytes = 8;
        let _ = serve_tcp(addr, config);
    });

    let mut stream = None;
    for _ in 0..100 {
        match std::net::TcpStream::connect(addr) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    let mut stream = stream.unwrap();
    stream.write_all(&1024u32.to_le_bytes()).unwrap();
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).unwrap();
    let len = u32::from_le_bytes(len) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).unwrap();
    let response = BinaryResponse::parse(&payload).unwrap();
    assert_eq!(response.envelope.status, Status::Error);
    drop(stream);
    drop(handle);
}

#[test]
fn tcp_server_read_timeout_closes_stalled_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let handle = thread::spawn(move || {
        let mut config = ServerConfig::new(Arc::new(BinbitBackend));
        config.read_timeout = Some(Duration::from_millis(50));
        config.write_timeout = Some(Duration::from_millis(500));
        let _ = serve_tcp(addr, config);
    });

    let mut stream = None;
    for _ in 0..100 {
        match std::net::TcpStream::connect(addr) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut stream = stream.unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut byte = [0u8; 1];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock
            ) => {}
        other => panic!("expected stalled connection to close or time out, got {other:?}"),
    }
    drop(stream);
    drop(handle);
}

#[test]
fn tcp_server_replaces_oversized_response_with_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let handle = thread::spawn(move || {
        let mut config = ServerConfig::new(Arc::new(BinbitBackend));
        config.max_response_bytes = 15;
        let _ = serve_tcp(addr, config);
    });

    let mut builder = ExprBuilder::new();
    let t = builder.bool_true().unwrap();
    builder.assert(t).unwrap();
    let request = builder.build_solve_request(123, 0, false, false).unwrap();
    let frame = le::encode_transport_frame(&request).unwrap();

    let mut stream = None;
    for _ in 0..100 {
        match std::net::TcpStream::connect(addr) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    let mut stream = stream.unwrap();
    stream.write_all(&frame).unwrap();
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).unwrap();
    let len = u32::from_le_bytes(len) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).unwrap();
    let response = BinaryResponse::parse(&payload).unwrap();
    assert_eq!(response.envelope.status, Status::Error);
    drop(stream);
    drop(handle);
}
