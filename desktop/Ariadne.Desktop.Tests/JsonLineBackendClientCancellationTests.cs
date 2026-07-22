using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class JsonLineBackendClientCancellationTests
{
    [Fact]
    public async Task OptionalCommandDoesNotConvertCallerCancellationToDefaultValue()
    {
        using var client = new JsonLineBackendClient(null);
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(
            () => client.GetAppStatusAsync(cancellation.Token));
    }
}
