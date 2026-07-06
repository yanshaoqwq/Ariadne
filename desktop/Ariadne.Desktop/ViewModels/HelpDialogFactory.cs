using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

internal static class HelpDialogFactory
{
    public static ConfirmDialogViewModel CreateTutorialDialog(DisplayNameService displayNames)
    {
        return CreateInfoDialog(
            displayNames.Text("ui.tutorial.title"),
            TutorialMessage(displayNames),
            displayNames);
    }

    public static ConfirmDialogViewModel CreateVersionDialog(DisplayNameService displayNames, string versionText)
    {
        var message = string.Join(Environment.NewLine + Environment.NewLine, new[]
        {
            versionText,
            displayNames.Text("ui.version.channel"),
            displayNames.Text("ui.version.tutorial"),
            TutorialMessage(displayNames),
        });
        return CreateInfoDialog(displayNames.Text("ui.version.title"), message, displayNames);
    }

    public static ConfirmDialogViewModel CreateFeedbackDialog(DisplayNameService displayNames)
    {
        return CreateInfoDialog(
            displayNames.Text("ui.feedback.title"),
            displayNames.Text("ui.feedback.message"),
            displayNames);
    }

    private static ConfirmDialogViewModel CreateInfoDialog(string title, string message, DisplayNameService displayNames)
    {
        return new ConfirmDialogViewModel(
            title,
            message,
            new[]
            {
                new DialogButton(displayNames.Text("ui.common.close"), DialogButtonVariant.Primary, 0),
            })
        {
            CancelResultIndex = 0,
        };
    }

    private static string TutorialMessage(DisplayNameService displayNames)
    {
        return string.Join(Environment.NewLine + Environment.NewLine, new[]
        {
            displayNames.Text("ui.tutorial.step.workspace"),
            displayNames.Text("ui.tutorial.step.works"),
            displayNames.Text("ui.tutorial.step.confirmations"),
            displayNames.Text("ui.tutorial.step.settings"),
        });
    }
}
