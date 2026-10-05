namespace Dosi.Tracker.Projects;

/// <summary>Limits of a project's descriptive fields (shared contract §2), shared by the DTOs, the service rules and
/// the database columns.</summary>
public static class ProjectConsts
{
    public const int MaxTitleLength = 128;

    /// <summary>A project colour is a hex colour only (<see cref="ColorPattern"/>): the web app renders it as a style
    /// on project pages, where any other value could make every member's browser call a third party.</summary>
    public const string ColorPattern = "^#[0-9a-fA-F]{6}$";

    /// <summary>Length of a "#RRGGBB" colour.</summary>
    public const int MaxColorLength = 7;

    public const string DefaultColor = "#006bff";

    /// <summary>The description is returned in every project list, so it is bounded.</summary>
    public const int MaxDescriptionLength = 2048;
}
