use std::path::PathBuf;

use rumba_core::{expr::Expr as RumbaExpr, parser::parse_expr};
use smt_server::{handle_binary_frame, RumbaBackend};
use smt_wire::raw::{
    tag, BinaryResponse, BlobRef, ExprBuilder, ExprView, NodeRef, SimplifyBlock, Status,
};

const WIDTH: u32 = 64;
const BITS: u8 = 64;
const MASK: u64 = mask_for_width(WIDTH);
const SEMANTIC_TEST_COUNT: usize = 200;

fn build_rumba_expr(
    builder: &mut ExprBuilder,
    expr: &RumbaExpr,
    width: u32,
) -> smt_wire::Result<NodeRef> {
    match expr {
        RumbaExpr::Var(var) => builder.bv_var(&format!("v{}", var.0), width),
        RumbaExpr::Const(c) => builder.bv_const(c & mask_for_width(width), width),
        RumbaExpr::Not(child) => {
            let child = build_rumba_expr(builder, child, width)?;
            builder.bv_not(child)
        }
        RumbaExpr::Scale(c, child) => {
            let coeff = builder.bv_const(c & mask_for_width(width), width)?;
            let child = build_rumba_expr(builder, child, width)?;
            builder.bv_mul(coeff, child)
        }
        RumbaExpr::And(children) => fold_rumba_children(
            builder,
            children,
            width,
            mask_for_width(width),
            |b, a, c| b.bv_and(a, c),
        ),
        RumbaExpr::Or(children) => {
            fold_rumba_children(builder, children, width, 0, |b, a, c| b.bv_or(a, c))
        }
        RumbaExpr::Xor(children) => {
            fold_rumba_children(builder, children, width, 0, |b, a, c| b.bv_xor(a, c))
        }
        RumbaExpr::Add(children) => {
            fold_rumba_children(builder, children, width, 0, |b, a, c| b.bv_add(a, c))
        }
        RumbaExpr::Mul(children) => {
            fold_rumba_children(builder, children, width, 1, |b, a, c| b.bv_mul(a, c))
        }
    }
}

fn fold_rumba_children(
    builder: &mut ExprBuilder,
    children: &[RumbaExpr],
    width: u32,
    identity: u64,
    mut op: impl FnMut(&mut ExprBuilder, NodeRef, NodeRef) -> smt_wire::Result<NodeRef>,
) -> smt_wire::Result<NodeRef> {
    let mut iter = children.iter();
    let Some(first) = iter.next() else {
        return builder.bv_const(identity, width);
    };
    let mut acc = build_rumba_expr(builder, first, width)?;
    for child in iter {
        let rhs = build_rumba_expr(builder, child, width)?;
        acc = op(builder, acc, rhs)?;
    }
    Ok(acc)
}

fn simplify_via_rumba_backend(expr: &RumbaExpr) -> SimplifyBlock {
    let mut builder = ExprBuilder::new();
    let target = build_rumba_expr(&mut builder, expr, WIDTH).unwrap();
    let request = builder.build_simplify_request(0x5255_4d42, target).unwrap();
    let response = BinaryResponse::parse(
        &handle_binary_frame(&request, &RumbaBackend)
            .unwrap()
            .encode()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response.envelope.status, Status::Simplified);
    SimplifyBlock::decode(&response.payload).unwrap()
}

#[derive(Clone, Copy)]
enum EvalNode {
    Var { id: usize, mask: u64 },
    Const(u64),
    Not { child: usize, mask: u64 },
    Neg { child: usize, mask: u64 },
    And { lhs: usize, rhs: usize, mask: u64 },
    Or { lhs: usize, rhs: usize, mask: u64 },
    Xor { lhs: usize, rhs: usize, mask: u64 },
    Add { lhs: usize, rhs: usize, mask: u64 },
    Sub { lhs: usize, rhs: usize, mask: u64 },
    Mul { lhs: usize, rhs: usize, mask: u64 },
}

struct CompiledWireExpr {
    nodes: Vec<EvalNode>,
    target: usize,
}

impl CompiledWireExpr {
    fn compile(view: &ExprView<'_>, target: NodeRef) -> smt_wire::Result<Self> {
        assert!(target.is_bv(), "dataset simplification target must be BV");
        let mut nodes = Vec::with_capacity(view.node_count() as usize);
        for index in 0..view.node_count() {
            let node = view.node(index)?;
            let mask = mask_for_width(node.width);
            let compiled = match node.tag {
                tag::BV_VAR => {
                    let name = view.blob_str(BlobRef::from_payload(node.payload), "BV variable")?;
                    let id = name
                        .strip_prefix('v')
                        .and_then(|suffix| suffix.parse::<usize>().ok())
                        .ok_or_else(|| {
                            smt_wire::WireError::invalid("test variable", name.to_owned())
                        })?;
                    EvalNode::Var { id, mask }
                }
                tag::BV_CONST => EvalNode::Const(node.payload & mask),
                tag::BV_NOT => EvalNode::Not {
                    child: child_index(view, &node, 0)?,
                    mask,
                },
                tag::BV_NEG => EvalNode::Neg {
                    child: child_index(view, &node, 0)?,
                    mask,
                },
                tag::BV_AND => EvalNode::And {
                    lhs: child_index(view, &node, 0)?,
                    rhs: child_index(view, &node, 1)?,
                    mask,
                },
                tag::BV_OR => EvalNode::Or {
                    lhs: child_index(view, &node, 0)?,
                    rhs: child_index(view, &node, 1)?,
                    mask,
                },
                tag::BV_XOR => EvalNode::Xor {
                    lhs: child_index(view, &node, 0)?,
                    rhs: child_index(view, &node, 1)?,
                    mask,
                },
                tag::BV_ADD => EvalNode::Add {
                    lhs: child_index(view, &node, 0)?,
                    rhs: child_index(view, &node, 1)?,
                    mask,
                },
                tag::BV_SUB => EvalNode::Sub {
                    lhs: child_index(view, &node, 0)?,
                    rhs: child_index(view, &node, 1)?,
                    mask,
                },
                tag::BV_MUL => EvalNode::Mul {
                    lhs: child_index(view, &node, 0)?,
                    rhs: child_index(view, &node, 1)?,
                    mask,
                },
                _ => {
                    return Err(smt_wire::WireError::invalid(
                        "test wire evaluator",
                        format!("unsupported returned tag {}", node.tag),
                    ))
                }
            };
            nodes.push(compiled);
        }
        Ok(Self {
            nodes,
            target: target.index() as usize,
        })
    }

    fn eval(&self, vars: &[u64]) -> u64 {
        let mut values = Vec::<u64>::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let value = match *node {
                EvalNode::Var { id, mask } => vars.get(id).copied().unwrap_or(0) & mask,
                EvalNode::Const(value) => value,
                EvalNode::Not { child, mask } => !values[child] & mask,
                EvalNode::Neg { child, mask } => values[child].wrapping_neg() & mask,
                EvalNode::And { lhs, rhs, mask } => (values[lhs] & values[rhs]) & mask,
                EvalNode::Or { lhs, rhs, mask } => (values[lhs] | values[rhs]) & mask,
                EvalNode::Xor { lhs, rhs, mask } => (values[lhs] ^ values[rhs]) & mask,
                EvalNode::Add { lhs, rhs, mask } => values[lhs].wrapping_add(values[rhs]) & mask,
                EvalNode::Sub { lhs, rhs, mask } => values[lhs].wrapping_sub(values[rhs]) & mask,
                EvalNode::Mul { lhs, rhs, mask } => values[lhs].wrapping_mul(values[rhs]) & mask,
            };
            values.push(value);
        }
        values[self.target]
    }
}

fn child_index(
    view: &ExprView<'_>,
    node: &smt_wire::raw::RawNode,
    offset: usize,
) -> smt_wire::Result<usize> {
    Ok(view.child_ref(node.children + offset as u32)?.index() as usize)
}

const fn mask_for_width(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else if width == 0 {
        0
    } else {
        (1u64 << width) - 1
    }
}

#[test]
fn rumba_simplify_protocol_uses_target_expression_not_assertions() {
    let expr = parse_expr("v0 + 0").unwrap();
    let block = simplify_via_rumba_backend(&expr);
    let view = ExprView::parse_and_validate(&block.expression).unwrap();
    let node = view.node(block.target_node.index()).unwrap();
    assert_eq!(node.tag, tag::BV_VAR);
}

#[test]
fn rumba_simplify_reuses_variable_names_across_wire_nodes() {
    let expr = parse_expr("v0 + (- v0)").unwrap();
    let block = simplify_via_rumba_backend(&expr);
    let view = ExprView::parse_and_validate(&block.expression).unwrap();
    let node = view.node(block.target_node.index()).unwrap();
    assert_eq!(node.tag, tag::BV_CONST);
    assert_eq!(node.payload, 0);
}

const INLINE_RUMBA_SAMPLES: &[(&str, &str)] = &[
    ("v0 + 0", "v0"),
    ("v0 + (- v0)", "0"),
    ("(v0 & v1) | (v0 & (~ v1))", "v0"),
];

#[test]
fn rumba_csv_dataset_samples_through_binary_simplify() {
    let full = std::env::var_os("SMT_SERVER_RUMBA_FULL_DATASET").is_some();
    if !full {
        for (line_index, (mba, ground_truth)) in INLINE_RUMBA_SAMPLES.iter().enumerate() {
            check_rumba_dataset_row("inline", line_index, mba, ground_truth);
        }
        return;
    }

    let dataset_dir = dataset_dir();
    let files = [
        "loki_tiny.csv",
        "mba_flatten.csv",
        "mba_obf_linear.csv",
        "mba_obf_nonlinear.csv",
        "neureduce.csv",
        "qsynth_ea.csv",
        "syntia.csv",
    ];

    for filename in files {
        let path = dataset_dir.join(filename);
        let csv = std::fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read {}: {err}", path.display());
        });
        eprintln!("checking {filename}");
        for (line_index, line) in csv
            .lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
        {
            let (mba, ground_truth) = line.split_once(',').unwrap_or_else(|| {
                panic!("{}:{}: expected two CSV columns", filename, line_index + 1)
            });
            check_rumba_dataset_row(filename, line_index, mba.trim(), ground_truth.trim());
        }
    }
}

fn check_rumba_dataset_row(filename: &str, line_index: usize, mba: &str, ground_truth: &str) {
    let mba = parse_expr(mba).unwrap_or_else(|err| {
        panic!(
            "{}:{}: failed to parse MBA: {err}",
            filename,
            line_index + 1
        )
    });
    let ground_truth = parse_expr(ground_truth).unwrap_or_else(|err| {
        panic!(
            "{}:{}: failed to parse ground truth: {err}",
            filename,
            line_index + 1
        )
    });
    let simplified = simplify_via_rumba_backend(&mba);
    let simplified_view = ExprView::parse_and_validate(&simplified.expression).unwrap();
    let compiled_simplified =
        CompiledWireExpr::compile(&simplified_view, simplified.target_node).unwrap();
    let var_count = mba
        .get_vars()
        .union(&ground_truth.get_vars())
        .map(|var| var.0)
        .max()
        .unwrap_or(0)
        + 1;
    let mut rng = 0x9e37_79b9_7f4a_7c15u64 ^ ((line_index as u64) << 32);
    for _ in 0..SEMANTIC_TEST_COUNT {
        let mut vars = Vec::with_capacity(var_count);
        for _ in 0..var_count {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            vars.push(rng & MASK);
        }
        let got = compiled_simplified.eval(&vars);
        let expected = ground_truth.eval(&vars, BITS);
        assert_eq!(
            got,
            expected,
            "{}:{} semantic mismatch vars={vars:?} gt={ground_truth}",
            filename,
            line_index + 1
        );
    }
}

fn dataset_dir() -> PathBuf {
    std::env::var_os("SMT_SERVER_RUMBA_DATASET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../rumba/third_party/dataset")
        })
}
