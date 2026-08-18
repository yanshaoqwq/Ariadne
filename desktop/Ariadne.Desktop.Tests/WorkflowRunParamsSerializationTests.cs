using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U156（P0 发布阻断）：桌面端「运行」按钮恒定失败——发出的是 <c>"variables": null</c>，
/// 后端拒收**整条请求**，报 `invalid ipc params: invalid type: null, expected a map`。
///
/// 这是产品的主功能：**点运行什么都不会发生**。
///
/// 缺陷形态是「匿名对象做不到按需加键」：
/// <code>variables = variables is { Count: &gt; 0 } ? variables : null</code>
/// 上方还配着「variables 为空时不发该键」的注释——**注释的意图对，代码做的正好相反**。
/// 匿名对象的属性集是**编译期固定**的，三元表达式只能改值、改不了「这个键在不在」，
/// 而 <c>System.Text.Json</c> 默认序列化 null 属性。
/// 后端 <c>RunWorkflowParams.variables</c> 是**非 <c>Option</c>** 的 <c>BTreeMap</c>
/// + <c>#[serde(default)]</c>，而 **<c>default</c> 只对「键缺失」生效、不接受显式 null**。
///
/// ⚠️ 旁边的 <c>start_node_id</c> 同样可能传 null 却没事，因为它是 <c>Option&lt;String&gt;</c>。
/// **两个字段并排、写法看着一样、一个能吃 null 一个不能**，这是本条最容易看漏的地方。
///
/// ⚠️ **判据必须落在真实出站的 JSON 字节上。**
/// 「命令能否执行」「有没有传 variables 参数」这类判据在缺陷版本下照样全绿——
/// 缺陷不在调用，在序列化。所以这里起一个**真进程**（回声脚本，读一行 stdin 就回一条响应），
/// 从它收到的原文里取 JSON 来断言。
/// 这也是 AGENTS.md 那条「任何跨进程边界至少要有一个真实进程的测试」的直接应用：
/// 本缺陷能活到发布前，正因为 Rust 侧测试进程内直调 `commands::`（不过 IPC 序列化层）、
/// 桌面侧测试用内存假客户端（不做 serde 反序列化），两边都绕开了出问题的那一层。
/// </summary>
[Collection("RealSidecar")]
public sealed class WorkflowRunParamsSerializationTests : IDisposable
{
    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-run-params-");

    public void Dispose()
    {
        try
        {
            _temp.Delete(recursive: true);
        }
        catch
        {
            // 清理失败不影响断言结论。
        }
    }

    /// <summary>
    /// **U156 主用例**：无变量时出站 JSON **不得含 `variables` 键**。
    ///
    /// 断言「键不存在」而不是「键的值不是 null」：后者可以被
    /// `variables = new Dictionary<string, object?>()`（发个空 map）蒙过去，
    /// 而那虽然后端能吃，却没解决「匿名对象改不了键集」这个根因——
    /// 下一个人照抄那种写法遇到真正非 Option 的字段还会再炸一次。
    /// </summary>
    [Fact]
    public async Task RunWorkflow_WithoutVariables_OmitsTheKeyEntirely()
    {
        var request = await CaptureRunWorkflowRequestAsync(variables: null);

        Assert.Equal("start_workflow", request.GetProperty("method").GetString());
        var parameters = request.GetProperty("params");

        Assert.False(
            parameters.TryGetProperty("variables", out var found),
            "无变量时必须**省略** variables 键。后端那个字段是非 Option 的 BTreeMap + "
            + $"#[serde(default)]，`default` 不接受显式 null ⇒ 整条请求被拒、运行按钮全废（U156）。"
            + $"实际发出：{found}");

        // 前置：这条请求本身是完整的，不是因为整体没序列化才「没有 variables」。
        Assert.Equal("wf-1", parameters.GetProperty("workflow_id").GetString());
    }

    /// <summary>
    /// **空字典**同样省略该键。
    ///
    /// 这一条不可省：生产的两条触发路径里，第二条正是
    /// `BuildStartVariables()` 在「无变量组」时返回**空字典**（不是 null）。
    /// 只测 null 会漏掉它——而那条路径覆盖「有 start 节点但没填变量」的全部工作流。
    /// </summary>
    [Fact]
    public async Task RunWorkflow_WithEmptyVariables_OmitsTheKeyToo()
    {
        var request = await CaptureRunWorkflowRequestAsync(
            new Dictionary<string, object?>(StringComparer.Ordinal));

        Assert.False(
            request.GetProperty("params").TryGetProperty("variables", out _),
            "空字典与「没有变量」同义，同样省略该键（U156 的第二条触发路径）");
    }

    /// <summary>
    /// 有变量时**必须**照发——否则「修好了 null」变成「变量根本没传下去」，
    /// 那是把一个显眼的报错换成一个静默的错误结果，更糟。
    /// </summary>
    [Fact]
    public async Task RunWorkflow_WithVariables_SendsThemThrough()
    {
        var request = await CaptureRunWorkflowRequestAsync(new Dictionary<string, object?>(StringComparer.Ordinal)
        {
            ["chapter"] = "chapter-01",
            ["retry"] = 2,
        });

        var variables = request.GetProperty("params").GetProperty("variables");
        Assert.Equal("chapter-01", variables.GetProperty("chapter").GetString());
        Assert.Equal(2, variables.GetProperty("retry").GetInt32());
    }

    /// <summary>
    /// <c>start_node_id</c> 为 null 时也省略——它虽是 <c>Option&lt;String&gt;</c>、发 null 不会炸，
    /// 但**「能吃 null」不该成为写法依据**：字段的 Rust 类型是后端实现细节，
    /// 前端按「没有值就不发键」统一处理，才不会在下一次某个字段从 Option 变成非 Option 时炸。
    /// </summary>
    [Fact]
    public async Task RunWorkflow_WithoutStartNode_OmitsThatKeyAsWell()
    {
        var request = await CaptureRunWorkflowRequestAsync(variables: null, startNodeId: null);

        Assert.False(
            request.GetProperty("params").TryGetProperty("start_node_id", out _),
            "没有起始节点时省略该键（它能吃 null，但不靠这一点）");
    }

    /// <summary>
    /// 起一个**真进程**当后端，捕获它从 stdin 收到的第一行请求原文。
    ///
    /// 回声脚本读一行就回一条固定成功响应——足以让客户端的调用完成，
    /// 而我们要的是它**写进管道的那一行字节**。
    /// 刻意不做假 <c>IAriadneBackendClient</c>：那样连序列化都不会发生，
    /// 而缺陷恰恰在序列化这一层（U156 就是被内存假客户端掩盖过去的）。
    ///
    /// ⚠️ 用 <c>sh</c> 脚本而非 .NET 子进程：只需要「读一行回一行」，
    /// 起第二个 .NET 进程要额外 100MB+ 内存，本机 3.8G 跑不动。
    /// Windows 上没有 <c>sh</c>，故整个用例族按平台跳过——**跳过要显式**，
    /// 不要让它在 Windows 上静默通过（那正是 U156 里「跳过被记成通过」那条教训）。
    /// </summary>
    private async Task<JsonElement> CaptureRunWorkflowRequestAsync(
        IReadOnlyDictionary<string, object?>? variables,
        string? startNodeId = null)
    {
        // ⚠️ 用 Assert.True 而非「静默 return」：**跳过被记成通过**正是 U156 能活到
        // 发布前的原因之一（sidecar 未编译时那几条跨进程测试直接 return，
        // 而 xUnit 把它记成绿）。本用例族的前提是 sh 存在——不满足就明确失败，
        // 让人去写 Windows 版本，而不是以为自己有覆盖。
        Assert.True(
            !OperatingSystem.IsWindows(),
            "回声后端用 sh 脚本实现；Windows 上需换 .cmd 或 PowerShell 版本再启用。"
            + "这里刻意 fail 而不是静默跳过——跳过会被记成通过，那样就等于没有覆盖（U156 的教训）");

        var capturePath = Path.Combine(_temp.FullName, "captured.jsonl");
        var scriptPath = Path.Combine(_temp.FullName, "echo-backend.sh");

        // 逐行读 stdin：把原文追加进捕获文件，然后回一条最小的成功响应。
        // `request_id` 必须回显，客户端按它匹配 pending 请求（否则调用永远不返回）。
        await File.WriteAllTextAsync(
            scriptPath,
            "#!/bin/sh\n"
            + "while IFS= read -r line; do\n"
            + $"  printf '%s\\n' \"$line\" >> '{capturePath}'\n"
            + "  id=$(printf '%s' \"$line\" | sed -n 's/.*\"request_id\":\"\\([^\"]*\\)\".*/\\1/p')\n"
            + "  printf '{\"request_id\":\"%s\",\"ok\":true,\"data\":{\"workflow_id\":\"wf-1\",\"run_id\":\"run-1\",\"status\":\"running\"}}\\n' \"$id\"\n"
            + "done\n");
        MakeExecutable(scriptPath);

        using var client = new JsonLineBackendClient(scriptPath);
        try
        {
            await client.RunWorkflowAsync("wf-1", startNodeId, variables);
        }
        catch (Exception)
        {
            // 回声后端只回一条最小响应，反序列化成 WorkflowRunStarted 可能不完整。
            // 本用例的判据是**出站字节**，出站已经发生，响应长什么样无关紧要。
        }

        Assert.True(File.Exists(capturePath), "回声后端没收到任何请求——客户端根本没把它启起来");
        var lines = await File.ReadAllLinesAsync(capturePath);

        // 客户端启动时可能先发若干握手/初始化请求，取 start_workflow 那一条。
        foreach (var line in lines)
        {
            using var document = JsonDocument.Parse(line);
            if (document.RootElement.TryGetProperty("method", out var method)
                && method.GetString() == "start_workflow")
            {
                return document.RootElement.Clone();
            }
        }

        throw new Xunit.Sdk.XunitException(
            "捕获到的请求里没有 start_workflow：\n" + string.Join('\n', lines));
    }

    /// <summary>
    /// 给脚本加可执行位。
    ///
    /// ⚠️ 用 <c>OperatingSystem.IsWindows()</c> 作为**运行时守卫**而不是
    /// <c>SupportedOSPlatform</c> 注解：CA1416 分析器只认前者这种它能静态推理的形状，
    /// 认不出上游那个 <c>Assert.True(!IsWindows)</c>（对它来说那只是个普通方法调用）。
    /// 加注解只会把警告从这里挪到调用点，不会消掉。
    /// </summary>
    private static void MakeExecutable(string path)
    {
        if (OperatingSystem.IsWindows())
        {
            // 上游已用 Assert 挡住 Windows，这里只是让分析器满意；真到不了。
            return;
        }
        File.SetUnixFileMode(
            path,
            UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);
    }
}
