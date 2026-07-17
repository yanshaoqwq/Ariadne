using AvaloniaEdit.Document;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 作品编辑器的单一连续文本缓冲。TextDocument 以内部分段树承载修改和撤销，
/// 页面只在保存、AI 请求或阅读投影刷新时物化完整字符串。
/// </summary>
public sealed class ContinuousDocumentBuffer
{
    private readonly ContinuousTextSnapshot _snapshot = new();

    public ContinuousDocumentBuffer()
    {
        Document = new TextDocument();
        Document.Changed += OnDocumentChanged;
    }

    public TextDocument Document { get; }

    /// <summary>
    /// 可供保存、加载竞争判定和后台状态检查读取的不可变正文快照。
    /// TextDocument 仍只由 UI 线程持有，业务层读取不再跨线程触碰编辑器对象。
    /// </summary>
    public int Length => _snapshot.Length;

    public string Text => _snapshot.Text;

    public event EventHandler? TextChanged;

    /// <summary>
    /// 外部替换使用最小公共前后缀差量，避免清空同一个 TextDocument 后重建，
    /// 从而让编辑器的全局光标、选区、滚动锚点和撤销栈继续由同一缓冲拥有。
    /// 加载/回滚等基线替换可显式清空撤销历史。
    /// </summary>
    public bool Replace(string content, bool resetUndoHistory)
    {
        content ??= string.Empty;
        var current = Text;
        if (string.Equals(current, content, StringComparison.Ordinal))
        {
            if (resetUndoHistory)
            {
                Document.UndoStack.ClearAll();
            }
            return false;
        }

        if (resetUndoHistory)
        {
            Document.Text = content;
            Document.UndoStack.ClearAll();
            return true;
        }

        var change = ContinuousTextChange.Between(current, content);
        Document.Replace(change.Offset, change.RemovalLength, change.Insertion);
        return true;
    }

    private void OnDocumentChanged(object? sender, DocumentChangeEventArgs e)
    {
        _snapshot.Replace(e.Offset, e.RemovalLength, e.InsertedText.Text);
        TextChanged?.Invoke(this, e);
    }
}

/// <summary>
/// TextDocument 的线程安全业务镜像。每次编辑只切分/拼接小片段，不物化整章；
/// 读取全文时一次性合并，并在片段过多时压实，避免长期退化。
/// </summary>
internal sealed class ContinuousTextSnapshot
{
    private const int CompactPieceThreshold = 2_048;
    private readonly object _gate = new();
    private List<TextPiece> _pieces = new();
    private int _length;

    public int Length => Volatile.Read(ref _length);

    public string Text
    {
        get
        {
            lock (_gate)
            {
                return CompactNoLock();
            }
        }
    }

    public void Replace(int offset, int removalLength, string insertion)
    {
        insertion ??= string.Empty;
        lock (_gate)
        {
            if (offset < 0 || removalLength < 0 || offset + removalLength > _length)
            {
                throw new ArgumentOutOfRangeException(nameof(offset), "continuous text change is outside the snapshot");
            }

            var updated = new List<TextPiece>(_pieces.Count + 2);
            AppendRangeNoLock(updated, 0, offset);
            if (insertion.Length > 0)
            {
                AppendCoalesced(updated, new TextPiece(insertion, 0, insertion.Length));
            }
            AppendRangeNoLock(updated, offset + removalLength, _length);
            _pieces = updated;
            Volatile.Write(ref _length, _length - removalLength + insertion.Length);

            if (_pieces.Count > CompactPieceThreshold)
            {
                CompactNoLock();
            }
        }
    }

    private void AppendRangeNoLock(List<TextPiece> destination, int rangeStart, int rangeEnd)
    {
        if (rangeEnd <= rangeStart)
        {
            return;
        }

        var position = 0;
        foreach (var piece in _pieces)
        {
            var pieceEnd = position + piece.Length;
            var intersectionStart = Math.Max(rangeStart, position);
            var intersectionEnd = Math.Min(rangeEnd, pieceEnd);
            if (intersectionEnd > intersectionStart)
            {
                AppendCoalesced(destination, new TextPiece(
                    piece.Source,
                    piece.Start + intersectionStart - position,
                    intersectionEnd - intersectionStart));
            }
            position = pieceEnd;
            if (position >= rangeEnd)
            {
                break;
            }
        }
    }

    private string CompactNoLock()
    {
        if (_length == 0)
        {
            _pieces.Clear();
            return string.Empty;
        }
        if (_pieces.Count == 1 && _pieces[0] is { Start: 0 } only && only.Length == only.Source.Length)
        {
            return only.Source;
        }

        var builder = new System.Text.StringBuilder(_length);
        foreach (var piece in _pieces)
        {
            builder.Append(piece.Source.AsSpan(piece.Start, piece.Length));
        }
        var text = builder.ToString();
        _pieces = new List<TextPiece> { new(text, 0, text.Length) };
        return text;
    }

    private static void AppendCoalesced(List<TextPiece> destination, TextPiece piece)
    {
        if (piece.Length == 0)
        {
            return;
        }
        if (destination.Count > 0)
        {
            var previous = destination[^1];
            if (ReferenceEquals(previous.Source, piece.Source)
                && previous.Start + previous.Length == piece.Start)
            {
                destination[^1] = previous with { Length = previous.Length + piece.Length };
                return;
            }
        }
        destination.Add(piece);
    }

    private readonly record struct TextPiece(string Source, int Start, int Length);
}

public readonly record struct ContinuousTextChange(int Offset, int RemovalLength, string Insertion)
{
    public static ContinuousTextChange Between(string current, string updated)
    {
        current ??= string.Empty;
        updated ??= string.Empty;

        var prefix = 0;
        var prefixLimit = Math.Min(current.Length, updated.Length);
        while (prefix < prefixLimit && current[prefix] == updated[prefix])
        {
            prefix++;
        }

        var currentSuffix = current.Length;
        var updatedSuffix = updated.Length;
        while (currentSuffix > prefix
               && updatedSuffix > prefix
               && current[currentSuffix - 1] == updated[updatedSuffix - 1])
        {
            currentSuffix--;
            updatedSuffix--;
        }

        return new ContinuousTextChange(
            prefix,
            currentSuffix - prefix,
            updated[prefix..updatedSuffix]);
    }
}
