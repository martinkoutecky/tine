use super::{IrAnswer, RESULT_BRIDGE_MAX_BYTES, RESULT_BRIDGE_MAX_ROWS};

/// Consume an IR answer into wire rows without cloning or I/O (O(1)). Refuses
/// exceeded results with store limits; other answer variants are unexpected.
impl TryFrom<IrAnswer> for tine_core::query::ir::QueryResult {
    type Error = String;
    fn try_from(answer: IrAnswer) -> Result<Self, Self::Error> {
        let IrAnswer::Result(result) = answer else {
            return Err("query-run: unexpected answer".into());
        };
        if result.exceeded {
            let complete = result.matched_total.unwrap_or(result.total);
            return Err(format!("result-too-large: {complete} matching rows; narrow the query or add a sample (construction limits: {RESULT_BRIDGE_MAX_ROWS} rows / {RESULT_BRIDGE_MAX_BYTES} bytes)"));
        }
        Ok(*result)
    }
}
