use std::sync::Arc;

use smt_wire::raw::{BinaryRequest, Command, SimplifyBlock, WireError};

use crate::backend::{Backend, QueryResult, QueryStatus, SolveContext};

/// Runs several simplifier backends in sequence, feeding each stage the
/// previous stage's output expression. A stage that declines (returns
/// `UNKNOWN`, errors, or produces an invalid artifact) is skipped and the
/// pipeline continues from the last good result.
///
/// This lets complementary simplifiers compose: Rumba's fast linear-MBA
/// rewriting first, CoBRA's deeper worklist pipeline on what remains.
#[derive(Clone)]
pub struct SimplifyChainBackend {
    stages: Vec<Arc<dyn Backend>>,
}

impl SimplifyChainBackend {
    pub fn new(stages: Vec<Arc<dyn Backend>>) -> Self {
        Self { stages }
    }

    pub fn stages(&self) -> &[Arc<dyn Backend>] {
        &self.stages
    }

    fn run(
        &self,
        request: &BinaryRequest,
        context: Option<&SolveContext>,
    ) -> smt_wire::Result<QueryResult> {
        let target = request
            .target_ref()
            .ok_or_else(|| WireError::invalid("simplify request", "missing target_node"))?;
        let mut current = SimplifyBlock {
            expression: request.expression.clone(),
            target_node: target,
        };
        for stage in &self.stages {
            if context.is_some_and(SolveContext::is_cancelled) {
                return Ok(cancelled_result());
            }
            let Ok(stage_request) = request_for_block(request, &current) else {
                break;
            };
            let result = match context {
                Some(context) => stage.handle_with_context(&stage_request, context),
                None => stage.handle(&stage_request),
            };
            let Ok(result) = result else {
                continue;
            };
            if result.status != QueryStatus::Simplified
                || result.validate_artifacts_for(&stage_request).is_err()
            {
                continue;
            }
            if let Some(block) = result.simplify {
                current = block;
            }
        }
        // Match the backend cancellation convention: a cancelled request
        // returns an inconclusive result, never a (vacuously valid) identity
        // simplification that a racing layer could adopt. Cancellation can
        // also arrive while the final stage runs, so check again on the way
        // out, not only before each stage.
        if context.is_some_and(SolveContext::is_cancelled) {
            return Ok(cancelled_result());
        }
        Ok(QueryResult::simplified(current))
    }
}

fn cancelled_result() -> QueryResult {
    QueryResult::unknown("simplify chain request cancelled before completion")
}

/// Rewrap a stage's output as the next stage's input request, keeping the
/// original envelope but dropping assertions: SIMPLIFY operates on the target
/// expression alone.
fn request_for_block(
    request: &BinaryRequest,
    block: &SimplifyBlock,
) -> smt_wire::Result<BinaryRequest> {
    BinaryRequest::new(
        request.envelope.request_id,
        Command::Simplify,
        request.envelope.flags,
        request.envelope.budget_ms,
        block.expression.clone(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Some(block.target_node),
    )
}

impl Backend for SimplifyChainBackend {
    fn name(&self) -> &'static str {
        "simplify-chain"
    }

    fn handle(&self, request: &BinaryRequest) -> smt_wire::Result<QueryResult> {
        match request.envelope.command {
            Command::Simplify => self.run(request, None),
            Command::Solve | Command::Minimize | Command::Maximize => Ok(QueryResult::unknown(
                "simplify-chain only supports SIMPLIFY",
            )),
        }
    }

    fn handle_with_context(
        &self,
        request: &BinaryRequest,
        context: &SolveContext,
    ) -> smt_wire::Result<QueryResult> {
        match request.envelope.command {
            Command::Simplify => self.run(request, Some(context)),
            Command::Solve | Command::Minimize | Command::Maximize => Ok(QueryResult::unknown(
                "simplify-chain only supports SIMPLIFY",
            )),
        }
    }
}
