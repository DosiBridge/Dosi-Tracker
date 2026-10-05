using System;
using System.Globalization;
using System.Linq;
using System.Text;

namespace Dosi.Tracker.Teams;

/// <summary>
/// User names for invited people. An email address is a valid user name only when every character is one Identity
/// allows (letters, digits and <c>-._@+</c> by default), which rules out valid addresses such as
/// <c>o'brien@acme.com</c>; those people get a user name derived from the address instead. Pure, so it is unit-tested
/// directly; uniqueness is checked by the caller.
/// </summary>
public static class InviteUserNames
{
    /// <summary>The stem used when nothing usable is left of the address.</summary>
    public const string FallbackStem = "member";

    /// <summary>Longest stem: leaves room for a numeric suffix within Identity's 256-character user-name limit.</summary>
    public const int MaxStemLength = 240;

    /// <summary>Whether Identity accepts <paramref name="userName"/> as is. An empty allow-list allows everything.</summary>
    public static bool IsAllowed(string userName, string? allowedCharacters)
    {
        return string.IsNullOrEmpty(allowedCharacters) || userName.All(allowedCharacters.Contains);
    }

    /// <summary>
    /// The allowed characters of the address's local part (before the last '@'), without '@' so the result can never
    /// equal somebody's email address, and without leading or trailing punctuation: <c>o'brien@acme.com</c> → <c>obrien</c>.
    /// </summary>
    public static string Stem(string email, string? allowedCharacters)
    {
        var at = email.LastIndexOf('@');
        var localPart = at > 0 ? email[..at] : email;

        var builder = new StringBuilder(localPart.Length);
        foreach (var character in localPart)
        {
            if (character != '@' && (string.IsNullOrEmpty(allowedCharacters) || allowedCharacters.Contains(character)))
            {
                builder.Append(character);
            }
        }

        var stem = builder.ToString().Trim('.', '-', '_', '+');
        if (stem.Length > MaxStemLength)
        {
            stem = stem[..MaxStemLength];
        }

        return stem.Length == 0 ? FallbackStem : stem;
    }

    /// <summary>The <paramref name="attempt"/>-th candidate (1-based): the stem itself, then stem2, stem3, …</summary>
    public static string Candidate(string stem, int attempt)
    {
        if (attempt < 1)
        {
            throw new ArgumentOutOfRangeException(nameof(attempt), attempt, "Attempts start at 1.");
        }

        return attempt == 1 ? stem : stem + attempt.ToString(CultureInfo.InvariantCulture);
    }
}
