using Ariadne.Desktop.ViewModels;
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
}
