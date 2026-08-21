namespace Ariadne.Desktop.ViewModels;

internal sealed record SettingsSaveAttempt(
    long Token,
    string Section,
    IReadOnlyDictionary<string, string> SubmittedValues);

internal sealed class SettingsDraftState
{
    private readonly Dictionary<string, string> _savedValues = new(StringComparer.Ordinal);
    private readonly Dictionary<string, string> _fieldSections = new(StringComparer.Ordinal);
    private readonly Dictionary<string, SettingsSaveAttempt> _saving = new(StringComparer.Ordinal);
    private readonly HashSet<string> _loadedSections = new(StringComparer.Ordinal);
    private long _nextToken;
    private long _loadGeneration;

    public long BeginLoad()
    {
        _loadGeneration++;
        _loadedSections.Clear();
        _savedValues.Clear();
        _fieldSections.Clear();
        _saving.Clear();
        return _loadGeneration;
    }

    public bool AcceptLoaded(
        long generation,
        string section,
        IReadOnlyDictionary<string, string> values)
    {
        if (generation != _loadGeneration)
        {
            return false;
        }

        foreach (var (field, value) in values)
        {
            _savedValues[field] = value;
            _fieldSections[field] = section;
        }
        _loadedSections.Add(section);
        return true;
    }

    public bool IsCurrentLoad(long generation) => generation == _loadGeneration;

    public long CurrentLoadGeneration => _loadGeneration;

    public bool IsLoaded(string section) => _loadedSections.Contains(section);

    public bool TryGetSavedValue(string section, string field, out string value)
    {
        if (_fieldSections.TryGetValue(field, out var owner)
            && string.Equals(owner, section, StringComparison.Ordinal)
            && _savedValues.TryGetValue(field, out var saved))
        {
            value = saved;
            return true;
        }

        value = string.Empty;
        return false;
    }

    /// <summary>
    /// 把当前值当作该分区的干净基线（「现在起没有未保存改动」）。
    ///
    /// U211-A：**刻意不再 `_loadedSections.Add(section)`**。原先无条件加，于是
    /// 「选中一个供应商」这个纯 UI 动作（`SelectProviderForEditing` 走到这里）
    /// 会把一个**读取失败**的分区伪装成已加载 ⇒ `CanSave` 转真 ⇒ 保存按钮点亮 ⇒
    /// 一保存就把项目里从未读到的四条默认模型路由清空。
    /// 缺陷版本里这条路够不着（列表是空的、选不了任何东西），U211-A 把列表填上
    /// 内置目录之后它就成了一条活的数据丢失路径。
    ///
    /// 「读取过」这个事实只由真实读取路径（<see cref="AcceptLoaded"/> /
    /// <see cref="ApplySavedValues"/>）建立，本方法只管基线、不管来源。
    /// 脏判定（<see cref="IsSectionDirty"/>）不看 `_loadedSections`，所以不受影响。
    /// </summary>
    /// <param name="markLoaded">
    /// 是否顺带把该分区标成「已读取过」。**默认 false**，只有真正代表一次成功读取的
    /// 调用方才该传 true（目前只有 `ApplyProviderConfigForTests` 这个模拟成功读取的钩子）。
    /// </param>
    public void SetBaseline(
        string section,
        IReadOnlyDictionary<string, string> values,
        bool markLoaded = false)
    {
        foreach (var field in _fieldSections
                     .Where(item => string.Equals(item.Value, section, StringComparison.Ordinal))
                     .Select(item => item.Key)
                     .ToArray())
        {
            _fieldSections.Remove(field);
            _savedValues.Remove(field);
        }
        foreach (var (field, value) in values)
        {
            _savedValues[field] = value;
            _fieldSections[field] = section;
        }
        if (markLoaded)
        {
            _loadedSections.Add(section);
        }
    }

    public void ApplySavedValues(string section, IReadOnlyDictionary<string, string> values)
    {
        foreach (var (field, value) in values)
        {
            _savedValues[field] = value;
            _fieldSections[field] = section;
        }
        _loadedSections.Add(section);
    }

    public bool IsSaving(string section) => _saving.ContainsKey(section);

    public bool IsAnySaving => _saving.Count > 0;

    public SettingsSaveAttempt? TryBeginSave(
        string section,
        IReadOnlyDictionary<string, string> submittedValues)
    {
        if (!IsLoaded(section) || _saving.ContainsKey(section))
        {
            return null;
        }

        var attempt = new SettingsSaveAttempt(
            ++_nextToken,
            section,
            new Dictionary<string, string>(submittedValues, StringComparer.Ordinal));
        _saving.Add(section, attempt);
        return attempt;
    }

    public bool CompleteSave(
        SettingsSaveAttempt attempt,
        IReadOnlyDictionary<string, string>? persistedValues = null)
    {
        if (!_saving.TryGetValue(attempt.Section, out var current)
            || current.Token != attempt.Token)
        {
            return false;
        }

        foreach (var (field, submittedValue) in attempt.SubmittedValues)
        {
            _savedValues[field] = persistedValues is not null
                && persistedValues.TryGetValue(field, out var persistedValue)
                    ? persistedValue
                    : submittedValue;
            _fieldSections[field] = attempt.Section;
        }
        _saving.Remove(attempt.Section);
        return true;
    }

    public void FailSave(SettingsSaveAttempt attempt)
    {
        if (_saving.TryGetValue(attempt.Section, out var current)
            && current.Token == attempt.Token)
        {
            _saving.Remove(attempt.Section);
        }
    }

    public bool IsDirty(IReadOnlyDictionary<string, string> currentValues) =>
        _savedValues.Any(item =>
            !currentValues.TryGetValue(item.Key, out var current)
            || !string.Equals(current, item.Value, StringComparison.Ordinal));

    public bool IsSectionDirty(
        string section,
        IReadOnlyDictionary<string, string> currentValues) =>
        _savedValues.Any(item =>
            _fieldSections.TryGetValue(item.Key, out var owner)
            && string.Equals(owner, section, StringComparison.Ordinal)
            && (!currentValues.TryGetValue(item.Key, out var current)
                || !string.Equals(current, item.Value, StringComparison.Ordinal)));

    public bool HasUnsubmittedChanges(IReadOnlyDictionary<string, string> currentValues)
    {
        foreach (var (field, savedValue) in _savedValues)
        {
            if (currentValues.TryGetValue(field, out var currentValue)
                && string.Equals(currentValue, savedValue, StringComparison.Ordinal))
            {
                continue;
            }

            var covered = _saving.Values.Any(attempt =>
                attempt.SubmittedValues.TryGetValue(field, out var submittedValue)
                && currentValues.TryGetValue(field, out var current)
                && string.Equals(current, submittedValue, StringComparison.Ordinal));
            if (!covered)
            {
                return true;
            }
        }
        return false;
    }
}
