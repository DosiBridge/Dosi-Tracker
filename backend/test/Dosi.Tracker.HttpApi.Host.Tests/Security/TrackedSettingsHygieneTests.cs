using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;
using System.Xml.Linq;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>
/// The repository is public: no settings file under <c>backend/</c> may carry an AI provider key, and the host's
/// git-ignored <c>appsettings.secrets.json</c> must never reach a publish folder (the Dockerfiles copy it into the
/// image, and it would override the container's environment). The secrets file itself and build output are skipped.
/// </summary>
public class TrackedSettingsHygieneTests
{
    private const string SecretsFileName = "appsettings.secrets.json";

    /// <summary>The shape of an OpenAI secret key ("sk-…", "sk-proj-…").</summary>
    private static readonly Regex OpenAiKeyPattern = new("sk-[A-Za-z0-9_-]{20,}", RegexOptions.CultureInvariant);

    private static readonly string BackendDirectory = Path.Combine(FindRepositoryRoot(), "backend");

    private static string FindRepositoryRoot()
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory); directory != null; directory = directory.Parent)
        {
            if (File.Exists(Path.Combine(directory.FullName, "backend", "Dosi.Tracker.slnx")))
            {
                return directory.FullName;
            }
        }

        throw new InvalidOperationException("No folder above the test output contains backend/Dosi.Tracker.slnx.");
    }

    public static IEnumerable<string> SettingsFiles()
    {
        return Directory.EnumerateFiles(BackendDirectory, "appsettings*.json", SearchOption.AllDirectories)
            .Where(path => !string.Equals(Path.GetFileName(path), SecretsFileName, StringComparison.OrdinalIgnoreCase))
            .Where(path => !Path.GetRelativePath(BackendDirectory, path)
                .Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
                .Any(segment => segment.Equals("bin", StringComparison.OrdinalIgnoreCase) ||
                                segment.Equals("obj", StringComparison.OrdinalIgnoreCase)))
            .OrderBy(path => path, StringComparer.Ordinal);
    }

    [Fact]
    public void The_Settings_Files_Are_Found()
    {
        var relative = SettingsFiles().Select(path => Path.GetRelativePath(BackendDirectory, path).Replace('\\', '/')).ToList();

        relative.ShouldContain("src/Dosi.Tracker.HttpApi.Host/appsettings.json");
        relative.ShouldContain("src/Dosi.Tracker.DbMigrator/appsettings.json");
        relative.ShouldNotContain(path => path.EndsWith(SecretsFileName, StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public void No_Settings_File_Carries_An_Ai_Api_Key()
    {
        var offenders = new List<string>();
        foreach (var path in SettingsFiles())
        {
            using var document = JsonDocument.Parse(File.ReadAllText(path), new JsonDocumentOptions
            {
                CommentHandling = JsonCommentHandling.Skip,
                AllowTrailingCommas = true
            });

            // Not even an empty placeholder: the key is supplied only by AI__OpenAI__ApiKey, the secrets file or a vault.
            if (FindProperty(document.RootElement, "AI", "OpenAI", "ApiKey") != null)
            {
                offenders.Add(Path.GetRelativePath(BackendDirectory, path));
            }
        }

        offenders.ShouldBeEmpty();
    }

    [Fact]
    public void No_Settings_File_Contains_Something_Shaped_Like_An_OpenAi_Key()
    {
        // The file names only, never the matched text: a real key must not end up in the test log either.
        SettingsFiles()
            .Where(path => OpenAiKeyPattern.IsMatch(File.ReadAllText(path)))
            .Select(path => Path.GetRelativePath(BackendDirectory, path))
            .ShouldBeEmpty();
    }

    [Theory]
    // Split literals: the test source itself never contains anything shaped like a key, for secret scanners' sake.
    [InlineData("sk" + "-abcdefghijklmnopqrstuvwxyz", true)]
    [InlineData("sk" + "-proj-ABC_def-0123456789xyzXYZ", true)]
    [InlineData("sk-short", false)]
    [InlineData("test-not-a-key", false)]
    public void The_Key_Pattern_Matches_OpenAi_Keys_Only(string value, bool matches)
    {
        OpenAiKeyPattern.IsMatch(value).ShouldBe(matches);
    }

    [Fact]
    public void The_Host_Never_Publishes_Its_Secrets_File()
    {
        var project = XDocument.Load(Path.Combine(BackendDirectory, "src", "Dosi.Tracker.HttpApi.Host", "Dosi.Tracker.HttpApi.Host.csproj"));
        var items = project.Descendants().Where(e => (string?)e.Attribute("Include") == SecretsFileName ||
                                                     (string?)e.Attribute("Remove") == SecretsFileName).ToList();

        // The Web SDK's default globs would publish it (as Content); it is removed from them and re-added with the rule.
        items.ShouldContain(e => e.Name.LocalName == "None" && (string?)e.Attribute("Remove") == SecretsFileName);
        items.ShouldContain(e => e.Name.LocalName == "Content" && (string?)e.Attribute("Remove") == SecretsFileName);
        var content = items.Where(e => e.Name.LocalName == "Content" && (string?)e.Attribute("Include") == SecretsFileName)
            .ShouldHaveSingleItem();
        ((string?)content.Attribute("Condition")).ShouldBe($"Exists('{SecretsFileName}')");
        content.Elements().Single(e => e.Name.LocalName == "CopyToPublishDirectory").Value.ShouldBe("Never");
    }

    /// <summary>The property at <paramref name="path"/>, matched case-insensitively like configuration keys.</summary>
    private static JsonElement? FindProperty(JsonElement element, params string[] path)
    {
        var current = element;
        foreach (var name in path)
        {
            if (current.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            var matches = current.EnumerateObject()
                .Where(p => string.Equals(p.Name, name, StringComparison.OrdinalIgnoreCase))
                .Select(p => p.Value)
                .ToList();
            if (matches.Count == 0)
            {
                return null;
            }

            current = matches[^1];
        }

        return current;
    }
}
