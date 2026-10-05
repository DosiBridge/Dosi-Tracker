using System;
using System.Linq;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Reporting;

/// <summary>
/// The contract §8 app identity. These cases mirror the web's canonicalAppName tests (frontend/src/lib/apps.test.ts),
/// so the reports' display names and the web's always agree.
/// </summary>
public class AppNamesTests
{
    [Theory]
    [InlineData("chrome.exe", "Google Chrome")]
    [InlineData("Google Chrome", "Google Chrome")]
    [InlineData("msedge.exe", "Microsoft Edge")]
    [InlineData("Code.exe", "Visual Studio Code")]
    [InlineData("Code", "Visual Studio Code")]
    [InlineData("firefox.exe", "Firefox")]
    [InlineData("slack.exe", "Slack")]
    [InlineData("teams.exe", "Microsoft Teams")]
    [InlineData("ms-teams.exe", "Microsoft Teams")]
    [InlineData("outlook.exe", "Outlook")]
    [InlineData("winword.exe", "Word")]
    [InlineData("excel.exe", "Excel")]
    [InlineData("powerpnt.exe", "PowerPoint")]
    [InlineData("explorer.exe", "File Explorer")]
    [InlineData("Finder", "Finder")]
    [InlineData("WindowsTerminal.exe", "Windows Terminal")]
    [InlineData("Terminal", "Terminal")]
    [InlineData("iTerm2", "iTerm2")]
    [InlineData("devenv.exe", "Visual Studio")]
    [InlineData("idea64.exe", "IntelliJ IDEA")]
    [InlineData("rider64.exe", "Rider")]
    [InlineData("figma.exe", "Figma")]
    [InlineData("Figma", "Figma")]
    [InlineData("zoom.exe", "Zoom")]
    [InlineData("zoom.us", "Zoom")]
    [InlineData("notepad.exe", "Notepad")]
    [InlineData("ApplicationFrameHost.exe", "Windows app")]
    [InlineData("LockApp.exe", "Lock screen")]
    public void Canonical_Should_Apply_The_Contract_Alias_Table(string raw, string expected)
    {
        AppNames.Canonical(raw).ShouldBe(expected);
    }

    [Fact]
    public void Canonical_Should_Match_Aliases_And_The_Exe_Suffix_Case_Insensitively_Ignoring_Surrounding_Whitespace()
    {
        AppNames.Canonical("CHROME.EXE").ShouldBe("Google Chrome");
        AppNames.Canonical("  WindowsTerminal.exe ").ShouldBe("Windows Terminal");
        AppNames.Canonical("code.Exe").ShouldBe("Visual Studio Code");
        AppNames.Canonical("ITERM2").ShouldBe("iTerm2");
    }

    [Fact]
    public void Canonical_Should_Title_Case_Any_Other_Stem_Keeping_The_Capitals_It_Already_Has()
    {
        AppNames.Canonical("myapp.exe").ShouldBe("Myapp");
        AppNames.Canonical("Antigravity IDE.exe").ShouldBe("Antigravity IDE");
        AppNames.Canonical("antigravity ide.exe").ShouldBe("Antigravity Ide");
        AppNames.Canonical("WhatsApp.exe").ShouldBe("WhatsApp");
        AppNames.Canonical("OneDrive.exe").ShouldBe("OneDrive");
        AppNames.Canonical("notepad++.exe").ShouldBe("Notepad++");
        AppNames.Canonical("obs64.exe").ShouldBe("Obs64");
        AppNames.Canonical("ß-tool.exe").ShouldBe("ß-tool"); // no single-letter upper case: kept as is
    }

    [Fact]
    public void Canonical_Should_Collapse_Runs_Of_Whitespace_And_Strip_Every_Trailing_Exe()
    {
        AppNames.Canonical("my   tool.exe").ShouldBe("My Tool");
        AppNames.Canonical("my\ttool app.exe").ShouldBe("My Tool App");
        AppNames.Canonical("tool.exe.exe").ShouldBe("Tool");
        AppNames.Canonical("tool .exe").ShouldBe("Tool");
        AppNames.Canonical("﻿slack.exe").ShouldBe("Slack");
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData(".exe")]
    [InlineData(" .EXE .exe ")]
    public void Canonical_Should_Be_Empty_For_A_Missing_Name(string? raw)
    {
        AppNames.Canonical(raw).ShouldBe(string.Empty);
        AppNames.GroupKey(raw).ShouldBe(string.Empty);
    }

    [Fact]
    public void Canonical_Should_Leave_Canonical_Names_Exactly_As_They_Are()
    {
        foreach (var name in new[]
                 {
                     "Windows app", "Lock screen", "iTerm2", "IntelliJ IDEA", "Visual Studio Code", "PowerPoint",
                     "Unknown app", "Other apps"
                 })
        {
            AppNames.Canonical(name).ShouldBe(name);
        }
    }

    [Fact]
    public void Canonical_Should_Be_Idempotent()
    {
        var random = new Random(20261005);
        const string alphabet = "abcXYZ .-+_ßéEXEexe\t";
        var samples = Enumerable.Range(0, 500)
            .Select(_ => new string(Enumerable.Range(0, random.Next(0, 16)).Select(_ => alphabet[random.Next(alphabet.Length)]).ToArray()))
            .Concat(new[] { "chrome.exe", "Code", "zoom.us", "ApplicationFrameHost.exe", "LockApp.exe", "iTerm2", "ß-tool.exe" });

        foreach (var raw in samples)
        {
            var once = AppNames.Canonical(raw);
            AppNames.Canonical(once).ShouldBe(once, $"canonicalising '{raw}' twice");
        }
    }

    [Fact]
    public void GroupKey_Should_Join_Every_Spelling_Of_One_App()
    {
        AppNames.GroupKey("chrome.exe").ShouldBe("google chrome");
        AppNames.GroupKey("Google Chrome").ShouldBe(AppNames.GroupKey("CHROME.EXE"));
        AppNames.GroupKey("MyTool.exe").ShouldBe(AppNames.GroupKey("mytool.EXE"));
        AppNames.GroupKey("Code").ShouldBe(AppNames.GroupKey("code.exe"));
    }
}
