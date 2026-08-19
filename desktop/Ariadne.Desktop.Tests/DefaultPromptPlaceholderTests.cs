using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U201-C：新建写作节点存的是**默认提示词占位符**（一行），不是 300~470 字全文。
///
/// 本文件回答两个问题：
/// 1. 拖一个写作节点上画布，<c>PromptTemplate</c> 里是那一行占位符吗？
///    （过去是全文副本 ⇒ 编辑框一进节点就被占满；工作流文件里存着副本 ⇒
///    官方后续调整默认提示词对已建节点无效。）
/// 2. 「作者没改过」与「作者改成了别的」分得开吗？
///    分不开的两种后果都很坏：把改过的当默认会覆盖作者的稿子；
///    把默认的当改过会让每次切界面语言都多出一份假改动。
///
/// ⚠️ 断言里**不硬写任何语言的占位符文案**，一律运行时从语言包读那个 key。
/// 这样用例跟着补译走——语言包改了不用改用例；也顺带验证并集真是从资源现算的。
/// </summary>
public sealed class DefaultPromptPlaceholderTests
{
    /// <summary>9 个写作 agent 的节点类型名（与后端 <c>WritingAgentKind::node_type</c> 一致）。</summary>
    private static readonly string[] WritingNodeTypes =
    {
        "outliner", "designer", "planner", "detail",
        "writer", "critic", "prudent", "polisher", "summarizer",
    };

    private static string PlaceholderKey(string nodeType) =>
        $"ui.prompt.default_placeholder.{nodeType}";

    [Fact]
    public void NewWritingNode_StoresOneLinePlaceholder_NotTheFullPromptBody()
    {
        var names = DisplayNameService.LoadDefault();
        PromptCatalog.ResetCacheForTests();

        foreach (var nodeType in WritingNodeTypes)
        {
            var placeholder = PromptCatalog.ResolveNodePromptPlaceholder(nodeType, names.Text);
            var fullBody = PromptCatalog.ResolveNodePrompt(nodeType);

            Assert.False(
                string.IsNullOrWhiteSpace(placeholder),
                $"{nodeType} 没有占位符：`{PlaceholderKey(nodeType)}` 缺 key 或值为空。"
                + "缺 key 时新建节点的提示词会是空的，作者拿不到默认角色设定。");

            // 一行：占位符的全部意义就是让编辑框保持一行、作者能看见并照抄语法。
            Assert.DoesNotContain('\n', placeholder);

            // 形态必须是 {{...}}——后端就按这个记号扫。
            Assert.StartsWith("{{", placeholder, StringComparison.Ordinal);
            Assert.EndsWith("}}", placeholder, StringComparison.Ordinal);

            // 关键：**不是全文**。这条就是本次改动的产品判据。
            Assert.NotEqual(fullBody, placeholder);
            Assert.True(
                placeholder.Length < 80,
                $"{nodeType} 的占位符有 {placeholder.Length} 字，看起来是全文而不是一行占位符"
                + $"（全文 {fullBody.Length} 字）。");

            // 占位符里必须是那个 key 的值本身，而不是 `[key]` 自查标记。
            var literal = names.Text(PlaceholderKey(nodeType));
            Assert.Equal("{{" + literal + "}}", placeholder);
        }
    }

    /// <summary>
    /// 「作者没改过」的判据必须容纳**任意一种语言**的写法。
    ///
    /// 占位符存在工作流文件里，不随界面语言改写：节点可能在中文界面建的、
    /// 现在切到了英文界面。只比当前语言会把「没改过」误判成「改成了别的」，
    /// 于是每次切语言都多出一份需要保存的假改动。
    ///
    /// 反过来也要成立：作者真改成别的内容（哪怕只是在占位符后面追加一句），
    /// 就**不能**再当默认处理——否则会覆盖他的稿子。
    /// </summary>
    [Fact]
    public void UnmodifiedCheck_AcceptsEveryLanguageSpelling_AndRejectsAuthorEdits()
    {
        var names = DisplayNameService.LoadDefault();

        foreach (var nodeType in WritingNodeTypes)
        {
            var literals = names.AllLanguageValues(PlaceholderKey(nodeType));
            Assert.True(
                literals.Count >= 2,
                $"{PlaceholderKey(nodeType)} 在语言包里只有 {literals.Count} 种写法；"
                + "并集是这条功能的地基，缺的那门语言里手打的写法会认不出。");

            foreach (var literal in literals)
            {
                // 每一种语言的写法都算「没改过」。
                Assert.True(
                    PromptCatalog.IsUnmodifiedDefaultPlaceholder("{{" + literal + "}}", literals),
                    $"{nodeType} 的某种语言写法没被认成默认值；"
                    + "切界面语言后会多出一份假改动。");

                // 空格差异也要认：中文里本来不需要空格，作者手打极可能少打那个。
                var noSpace = literal.Replace(" ", string.Empty);
                Assert.True(
                    PromptCatalog.IsUnmodifiedDefaultPlaceholder("{{" + noSpace + "}}", literals),
                    $"{nodeType} 去掉空格后的写法没被认成默认值；"
                    + "这条功能就是为「照抄」服务的，为一个空格判他写错等于白做。");
            }

            // 作者改过的一律不算默认——覆盖它等于删掉作者的稿子。
            var first = literals[0];
            Assert.False(
                PromptCatalog.IsUnmodifiedDefaultPlaceholder(
                    "{{" + first + "}}\n\n另外注意语气要克制。", literals),
                $"{nodeType}：作者在占位符后追加了内容，仍被当成默认值 —— 会被覆盖掉。");
            Assert.False(
                PromptCatalog.IsUnmodifiedDefaultPlaceholder(
                    PromptCatalog.ResolveNodePrompt(nodeType), literals),
                $"{nodeType}：存量工作流里的全文副本被当成了默认占位符。");
            Assert.False(
                PromptCatalog.IsUnmodifiedDefaultPlaceholder(string.Empty, literals));
        }
    }

    /// <summary>
    /// ⚠️ **判据必须落在真实 VM 的新建节点路径上**（`AddNodeAt` → `AddNode`）。
    ///
    /// 首版本用例只测 `PromptCatalog.ResolveNodePromptPlaceholder` 这个纯函数，
    /// 变异测试当场证明它是**空测**：把 `AddNode` 里的调用改回
    /// `ResolveNodePrompt(nodeType)`（写全文），用例照样全绿——
    /// 因为它压根没经过那个调用点。
    ///
    /// 所以这条用例走真实 VM：拖一个节点上画布，读它的 <c>PromptTemplate</c>。
    /// 这才是「作者拖一个写作节点上来，编辑框里是什么」这个产品判据本身。
    /// </summary>
    [Fact]
    public void DraggingAWritingNodeOntoTheCanvas_StoresThePlaceholder_NotTheFullBody()
    {
        var names = DisplayNameService.LoadDefault();
        PromptCatalog.ResetCacheForTests();

        foreach (var nodeType in WritingNodeTypes)
        {
            var vm = CreateWorkspaceVm(names);
            vm.AddNodeAt(nodeType, 120, 80);

            var node = Assert.IsType<WorkflowNodeViewModel>(vm.SelectedNode);
            Assert.True(
                node.ShowPromptEditor,
                $"{nodeType} 不是 agent 节点，本用例的前提不成立（是否改了 workflow_node_catalog.json？）");

            var fullBody = PromptCatalog.ResolveNodePrompt(nodeType);
            Assert.False(string.IsNullOrWhiteSpace(fullBody), $"{nodeType} 的默认提示词全文为空");

            // 目标判据：存的是一行占位符，**不是**全文副本。
            Assert.NotEqual(fullBody, node.PromptTemplate);
            Assert.DoesNotContain('\n', node.PromptTemplate);
            Assert.Equal(
                "{{" + names.Text(PlaceholderKey(nodeType)) + "}}",
                node.PromptTemplate);

            // 存进工作流文件的那份也必须是占位符（ToData 是落盘取值的那一步）。
            var persisted = node.ToData()["prompt_template"]?.ToString();
            Assert.Equal(node.PromptTemplate, persisted);
        }
    }

    /// <summary>HasProjectRoot=true；其余后端方法一律空实现。</summary>
    private static WorkspacePageViewModel CreateWorkspaceVm(DisplayNameService names) =>
        new(names, DispatchProxy.Create<IAriadneBackendClient, SoftBackendProxy>());

    private class SoftBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.ReturnType == typeof(bool) && targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            if (targetMethod.ReturnType == typeof(void) || targetMethod.ReturnType == typeof(Task))
            {
                return targetMethod.ReturnType == typeof(Task) ? Task.CompletedTask : null;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var t = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(t)
                    .Invoke(null, new object?[] { t.IsValueType ? Activator.CreateInstance(t) : null });
            }

            if (targetMethod.ReturnType.IsValueType)
            {
                return Activator.CreateInstance(targetMethod.ReturnType);
            }

            return null;
        }
    }
}
