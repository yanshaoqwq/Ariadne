namespace Ariadne.Desktop.ViewModels;

public sealed class PlaceholderPageViewModel : ViewModelBase
{
    public PlaceholderPageViewModel(string title, string description, string icon)
    {
        Title = title;
        Description = description;
        Icon = icon;
    }

    public string Title { get; }

    public string Description { get; }

    public string Icon { get; }
}
