using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class BoundedTextBufferTests
{
    [Fact]
    public void KeepsOnlyNewestDiagnosticTailWithinCapacity()
    {
        var buffer = new BoundedTextBuffer(12);

        buffer.AppendLine("first");
        buffer.AppendLine("second");
        buffer.AppendLine("latest");

        var text = buffer.Read();
        Assert.True(text.Length <= 12);
        Assert.DoesNotContain("first", text, StringComparison.Ordinal);
        Assert.EndsWith("latest" + Environment.NewLine, text, StringComparison.Ordinal);
    }
}
