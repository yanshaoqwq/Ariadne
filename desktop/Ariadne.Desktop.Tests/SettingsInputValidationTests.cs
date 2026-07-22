using Ariadne.Desktop.ViewModels;
using System.Reflection;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsInputValidationTests
{
    [Fact]
    public void InvalidNumberDoesNotFallbackToDefault()
    {
        var error = Assert.Throws<SettingsInputException>(() =>
            SettingsInputValidation.PositiveLong("not-a-timeout", "timeout"));

        Assert.Equal(SettingsInputFailure.Number, error.Failure);
        Assert.Equal("timeout", error.FieldKey);
    }

    [Fact]
    public void InvalidModelRowReportsOneBasedLine()
    {
        var error = Assert.Throws<SettingsInputException>(() =>
            SettingsInputValidation.Models("good,llm,4096,,\nbad,llm,unknown,,", "models"));

        Assert.Equal(SettingsInputFailure.ModelLine, error.Failure);
        Assert.Equal(2, error.Line);
    }

    [Fact]
    public void ValidModelsPreserveOptionalNumericColumns()
    {
        var models = SettingsInputValidation.Models(
            "writer,llm,8192,1.25,2.5\nembed,embedding,,,", "models");

        Assert.Equal(2, models.Count);
        Assert.Equal(8192, models[0].MaxContextTokens);
        Assert.Equal(1.25, models[0].InputCostPerMillionTokens);
        Assert.Null(models[1].MaxContextTokens);
    }

    [Theory]
    [InlineData("writer,unknown,,,")]
    [InlineData("writer,streaming,,,")]
    [InlineData("writer,tool_use,,,")]
    [InlineData("writer,llm,,,\nwriter,embedding,,,")]
    public void ModelsRejectUnknownCapabilitiesAndDuplicateIds(string input)
    {
        var error = Assert.Throws<SettingsInputException>(() =>
            SettingsInputValidation.Models(input, "models"));

        Assert.Equal(SettingsInputFailure.ModelLine, error.Failure);
    }

    [Fact]
    public void EmbeddingSelectionCannotRewriteExistingLlmCapability()
    {
        var models = new[]
        {
            new Ariadne.Desktop.Backend.ModelConfig("shared", "llm", null, null, null),
        };

        var merge = typeof(SettingsPageViewModel).GetMethod(
            "MergeEmbeddingModel",
            BindingFlags.NonPublic | BindingFlags.Static)!;
        var invocation = Assert.Throws<TargetInvocationException>(() =>
            merge.Invoke(null, new object[] { models, "shared" }));
        var error = Assert.IsType<SettingsInputException>(invocation.InnerException);

        Assert.Equal(SettingsInputFailure.ModelLine, error.Failure);
    }

    [Fact]
    public void RelativeDirectoryPathAcceptsAndNormalizesTrailingSeparator()
    {
        var paths = SettingsInputValidation.RelativePaths(".cache/\ndrafts\\", "ignored_paths");

        Assert.Equal(new[] { ".cache", "drafts" }, paths);
    }

    [Theory]
    [InlineData("/tmp/cache")]
    [InlineData("drafts/../secrets")]
    public void RelativeDirectoryPathRejectsAbsoluteAndParentEscape(string input)
    {
        var error = Assert.Throws<SettingsInputException>(() =>
            SettingsInputValidation.RelativePaths(input, "ignored_paths"));

        Assert.Equal(SettingsInputFailure.PathLine, error.Failure);
    }

    [Fact]
    public void AbsolutePathsRejectParentComponentsAndDuplicates()
    {
        var root = Path.GetTempPath().TrimEnd(Path.DirectorySeparatorChar);
        var parent = Path.Combine(root, "allowed", "..", "escaped");
        Assert.Throws<SettingsInputException>(() =>
            SettingsInputValidation.AbsolutePaths(parent, "roots"));

        var duplicate = Path.Combine(root, "allowed");
        var error = Assert.Throws<SettingsInputException>(() =>
            SettingsInputValidation.AbsolutePaths(
                duplicate + Environment.NewLine + duplicate,
                "roots"));
        Assert.Equal(2, error.Line);
    }

    [Fact]
    public void PathUniquenessFollowsWindowsCaseSemanticsOnly()
    {
        var root = Path.Combine(Path.GetTempPath(), "AriadneCaseSensitivePath");
        var first = Path.Combine(root, "Models");
        var second = Path.Combine(root, "models");
        var input = first + Environment.NewLine + second;

        if (OperatingSystem.IsWindows())
        {
            Assert.Throws<SettingsInputException>(() =>
                SettingsInputValidation.AbsolutePaths(input, "roots"));
            Assert.Equal(first, SettingsPageViewModel.AppendPathLine(first, second));
            return;
        }

        var paths = SettingsInputValidation.AbsolutePaths(input, "roots");
        Assert.Equal(new[] { Path.GetFullPath(first), Path.GetFullPath(second) }, paths);
        Assert.Equal(
            input,
            SettingsPageViewModel.AppendPathLine(first, second));
    }
}
