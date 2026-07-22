namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 异步选择/导航的单一代次所有者。旧请求可以完成清理，但不能再提交可见状态。
/// </summary>
internal sealed class RequestGenerationSession
{
    private readonly object _sync = new();
    private long _generation;
    private CancellationTokenSource _cancellation = new();

    public RequestGeneration Begin(long ownerGeneration = 0)
    {
        CancellationTokenSource previous;
        RequestGeneration request;
        lock (_sync)
        {
            previous = _cancellation;
            _cancellation = new CancellationTokenSource();
            request = new RequestGeneration(
                ++_generation,
                ownerGeneration,
                _cancellation.Token);
        }

        previous.Cancel();
        previous.Dispose();
        return request;
    }

    public bool IsCurrent(RequestGeneration request, long ownerGeneration = 0)
    {
        lock (_sync)
        {
            return request.Generation == _generation
                && request.OwnerGeneration == ownerGeneration
                && !request.CancellationToken.IsCancellationRequested;
        }
    }

    public void Invalidate()
    {
        CancellationTokenSource previous;
        lock (_sync)
        {
            _generation++;
            previous = _cancellation;
            _cancellation = new CancellationTokenSource();
        }

        previous.Cancel();
        previous.Dispose();
    }
}

internal readonly record struct RequestGeneration(
    long Generation,
    long OwnerGeneration,
    CancellationToken CancellationToken);
