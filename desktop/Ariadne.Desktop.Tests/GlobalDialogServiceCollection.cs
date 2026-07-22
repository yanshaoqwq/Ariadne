using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// DialogService.Current 是应用级单例；会替换它的测试必须共享同一互斥集合。
/// </summary>
[CollectionDefinition("GlobalDialogService", DisableParallelization = true)]
public sealed class GlobalDialogServiceCollection
{
}
