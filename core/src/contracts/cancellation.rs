/// workflow、LLM、Skill 与资源调度共用同一种取消令牌，避免各层维护互不关联
/// 的布尔状态。别名只表达该 token 在执行链中的职责，不创建第二套实现。
pub type ExecutionCancellation = super::CancellationToken;
