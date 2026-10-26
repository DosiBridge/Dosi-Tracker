using System;
using System.Text.RegularExpressions;

namespace Dosi.Tracker.AiAssist.Output;

/// <summary>
/// Words that judge people. AI text describes tracked data and never rates, ranks or accuses anyone, so output using
/// any of them is refused (whole words, any case).
/// </summary>
public static class AiBannedTerms
{
    public static readonly Regex Pattern = new(
        @"\b(?:lazy|slack(?:ing|er)|underperform\w*|poor\s+performance|low\s+performer|top\s+performer" +
        @"|unproductive\s+(?:employee|worker|member|person)|time\s+theft|cheat\w*|fraud\w*|faking|disciplin\w*" +
        @"|terminat\w*|should\s+be\s+fired|warning\s+letter|rank(?:ed|ing))\b",
        RegexOptions.Compiled | RegexOptions.CultureInvariant | RegexOptions.IgnoreCase,
        TimeSpan.FromSeconds(1));

    /// <summary>True when the text uses a banned term.</summary>
    public static bool Contains(string? text) => !string.IsNullOrEmpty(text) && Pattern.IsMatch(text);
}
