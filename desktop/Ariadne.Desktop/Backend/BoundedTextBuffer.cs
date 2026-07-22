using System.Text;

namespace Ariadne.Desktop.Backend;

internal sealed class BoundedTextBuffer
{
    private readonly int _capacity;
    private readonly StringBuilder _buffer = new();
    private readonly object _sync = new();

    public BoundedTextBuffer(int capacity)
    {
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(capacity);
        _capacity = capacity;
    }

    public void AppendLine(string line)
    {
        lock (_sync)
        {
            _buffer.AppendLine(line);
            var overflow = _buffer.Length - _capacity;
            if (overflow > 0)
            {
                _buffer.Remove(0, overflow);
            }
        }
    }

    public string Read()
    {
        lock (_sync)
        {
            return _buffer.ToString();
        }
    }

    public void Clear()
    {
        lock (_sync)
        {
            _buffer.Clear();
        }
    }
}
