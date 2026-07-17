using System.Collections.Concurrent;

namespace Ariadne.Desktop.Backend;

internal sealed class IpcResponseRouter
{
    private readonly ConcurrentDictionary<string, TaskCompletionSource<string>> _pending = new();

    public bool TryRegister(string requestId, out Task<string> response)
    {
        var completion = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        if (!_pending.TryAdd(requestId, completion))
        {
            response = Task.FromException<string>(new InvalidOperationException("duplicate request id"));
            return false;
        }

        response = completion.Task;
        return true;
    }

    public bool TryComplete(string requestId, string response)
    {
        return _pending.TryRemove(requestId, out var completion)
            && completion.TrySetResult(response);
    }

    public bool TryCancel(string requestId, CancellationToken cancellationToken)
    {
        return _pending.TryRemove(requestId, out var completion)
            && completion.TrySetCanceled(cancellationToken);
    }

    public void Remove(string requestId)
    {
        _pending.TryRemove(requestId, out _);
    }

    public void FailAll(Exception error)
    {
        foreach (var requestId in _pending.Keys)
        {
            if (_pending.TryRemove(requestId, out var completion))
            {
                completion.TrySetException(error);
            }
        }
    }
}
