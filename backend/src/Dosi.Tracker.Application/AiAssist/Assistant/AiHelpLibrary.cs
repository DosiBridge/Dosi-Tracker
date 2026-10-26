using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text.RegularExpressions;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.AiAssist.Assistant;

/// <summary>One help article (<c>AiAssist/Help/*.md</c>): a title, the keywords it is scored against, and its body
/// (at most 400 words; never a secret or a configuration key name).</summary>
public sealed record AiHelpArticle(string Title, IReadOnlyList<string> Keywords, string Body);

/// <summary>
/// The product's own how-to text for Ask AI's <c>get_help</c> tool (spec BE-11). Loads the embedded
/// <c>AiAssist/Help/*.md</c> articles once (a singleton: the files never change at runtime) and scores them against a
/// question by counting keyword hits, returning the top matches within a character budget that keeps the whole
/// result well under <c>AI:Assistant:MaxToolResultChars</c>. Carries no workspace data: this tool is the only one
/// every role, including a client, may call.
/// </summary>
public class AiHelpLibrary : ISingletonDependency
{
    /// <summary>Roughly 1500 tokens (the spec's cap), at the ~4-characters-per-token estimate used throughout AI
    /// assist (<see cref="Pipeline.AiInsightRunner.EstimateTokens"/>).</summary>
    public const int MaxCharacters = 1500 * 4;

    public const int MaxArticles = 3;

    private static readonly Regex FrontMatter = new(
        @"\A---\s*\ntitle:\s*(?<title>.+?)\s*\nkeywords:\s*(?<keywords>.+?)\s*\n---\s*\n(?<body>.*)\z",
        RegexOptions.Singleline | RegexOptions.Compiled);

    private static readonly Regex Word = new(@"[\p{L}\p{N}]+", RegexOptions.Compiled);

    private readonly IReadOnlyList<AiHelpArticle> _articles;

    public AiHelpLibrary()
        : this(typeof(AiHelpLibrary).Assembly)
    {
    }

    internal AiHelpLibrary(Assembly assembly)
    {
        _articles = assembly.GetManifestResourceNames()
            .Where(name => name.Contains(".AiAssist.Help.", StringComparison.Ordinal) && name.EndsWith(".md", StringComparison.Ordinal))
            .Select(name => Load(assembly, name))
            .Where(article => article != null)
            .Select(article => article!)
            .OrderBy(article => article.Title, StringComparer.Ordinal)
            .ToList();
    }

    public IReadOnlyList<AiHelpArticle> Articles => _articles;

    /// <summary>The best-matching articles for <paramref name="question"/>, most relevant first, kept within
    /// <see cref="MaxCharacters"/> in total; at most <see cref="MaxArticles"/>. A question that matches nothing
    /// returns the installation and overview articles, so the tool is never empty.</summary>
    public IReadOnlyList<AiHelpArticle> Find(string? question)
    {
        var words = new HashSet<string>(
            Word.Matches(question ?? string.Empty).Select(m => m.Value.ToLowerInvariant()),
            StringComparer.Ordinal);

        var ranked = _articles
            .Select(article => (article, score: Score(article, words)))
            .OrderByDescending(a => a.score)
            .ThenBy(a => a.article.Title, StringComparer.Ordinal)
            .ToList();

        var chosen = new List<AiHelpArticle>();
        var budget = MaxCharacters;
        foreach (var (article, score) in ranked)
        {
            if (chosen.Count >= MaxArticles)
            {
                break;
            }

            if (score == 0 && chosen.Count > 0)
            {
                break;
            }

            if (article.Body.Length > budget && chosen.Count > 0)
            {
                continue;
            }

            chosen.Add(article);
            budget -= article.Body.Length;
        }

        return chosen;
    }

    private static int Score(AiHelpArticle article, IReadOnlySet<string> words) =>
        article.Keywords.Count(keyword => words.Contains(keyword));

    private static AiHelpArticle? Load(Assembly assembly, string resourceName)
    {
        using var stream = assembly.GetManifestResourceStream(resourceName);
        if (stream == null)
        {
            return null;
        }

        using var reader = new StreamReader(stream);
        var text = reader.ReadToEnd();
        var match = FrontMatter.Match(text);
        if (!match.Success)
        {
            return null;
        }

        var keywords = match.Groups["keywords"].Value
            .Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries)
            .Select(k => k.ToLowerInvariant())
            .ToList();

        return new AiHelpArticle(match.Groups["title"].Value.Trim(), keywords, match.Groups["body"].Value.Trim());
    }
}
