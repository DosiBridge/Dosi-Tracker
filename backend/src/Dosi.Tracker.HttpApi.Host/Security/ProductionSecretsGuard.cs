using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Microsoft.Extensions.Configuration;

namespace Dosi.Tracker.Security;

/// <summary>
/// Production start-up check for secrets that must never come from the repository. Two rules per secret:
/// <list type="number">
/// <item>the value must not be a well-known ABP template default, and</item>
/// <item>the value must not be supplied by the tracked <c>appsettings.json</c> itself — it has to be
/// overridden by an environment variable, <c>appsettings.secrets.json</c>, a key vault, etc.</item>
/// </list>
/// Pure (no host dependencies) so it is unit-tested directly.
/// </summary>
public static class ProductionSecretsGuard
{
    public const string ConnectionStringKey = "ConnectionStrings:Default";
    public const string CertificatePassPhraseKey = "AuthServer:CertificatePassPhrase";
    public const string StringEncryptionPassPhraseKey = "StringEncryption:DefaultPassPhrase";

    /// <summary>The committed (tracked) settings file that must never be the source of a production secret.</summary>
    public const string TrackedSettingsFileName = "appsettings.json";

    private const string TemplateDbPasswordFragment = "Password=myPassword";
    private const string TemplateCertificatePassPhrase = "3f027835-7c52-4bf7-bc1f-77582ca37aef";
    private const string TemplateStringEncryptionPassPhrase = "5pGzpfXjidNXJVDa";

    private static readonly string[] SecretKeys =
    {
        ConnectionStringKey,
        CertificatePassPhraseKey,
        StringEncryptionPassPhraseKey
    };

    /// <summary>Returns a human-readable problem per offending secret; empty when the configuration is safe.</summary>
    public static IReadOnlyList<string> FindProblems(IConfiguration configuration)
    {
        var problems = new List<string>();

        if (configuration[ConnectionStringKey]?.Contains(TemplateDbPasswordFragment, StringComparison.Ordinal) == true)
        {
            problems.Add($"{ConnectionStringKey} uses the template database password");
        }

        if (configuration[CertificatePassPhraseKey] == TemplateCertificatePassPhrase)
        {
            problems.Add($"{CertificatePassPhraseKey} is the template default");
        }

        if (configuration[StringEncryptionPassPhraseKey] == TemplateStringEncryptionPassPhrase)
        {
            problems.Add($"{StringEncryptionPassPhraseKey} is the template default");
        }

        if (configuration is IConfigurationRoot root)
        {
            foreach (var key in SecretKeys)
            {
                if (IsSuppliedByTrackedSettingsFile(root, key))
                {
                    problems.Add($"{key} is read from the tracked {TrackedSettingsFileName}");
                }
            }
        }

        return problems;
    }

    /// <summary>True when the effective value of <paramref name="key"/> comes from the tracked
    /// <c>appsettings.json</c>, i.e. no later provider (env vars, secrets file, vault…) overrides it.</summary>
    public static bool IsSuppliedByTrackedSettingsFile(IConfigurationRoot root, string key)
    {
        // Later providers win, so the effective source is the LAST provider that has the key.
        var effective = root.Providers.LastOrDefault(p => p.TryGet(key, out _));

        return effective is FileConfigurationProvider fileProvider &&
               string.Equals(
                   Path.GetFileName(fileProvider.Source.Path),
                   TrackedSettingsFileName,
                   StringComparison.OrdinalIgnoreCase);
    }
}
