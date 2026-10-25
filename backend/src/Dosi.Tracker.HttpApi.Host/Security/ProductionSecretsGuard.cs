using System;
using System.Collections.Generic;
using System.Data.Common;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using Microsoft.Extensions.Configuration;

namespace Dosi.Tracker.Security;

/// <summary>
/// Start-up check, outside Development, for secrets that must never come from the repository. Rules:
/// <list type="number">
/// <item>a secret must not be a well-known value: the ABP template's passphrases, or a database password published in
/// this repository or used as a stock default (<see cref="KnownDatabasePasswords"/>);</item>
/// <item>a secret must not be supplied by the tracked <c>appsettings.json</c> itself — it has to be overridden by an
/// environment variable, <c>appsettings.secrets.json</c>, a key vault, etc. (the SMTP and S3 secrets only when they are
/// set at all);</item>
/// <item>the OpenIddict certificate must not be a localhost development certificate (as <c>dotnet dev-certs</c> makes)
/// or expired (<see cref="FindCertificateProblems"/>).</item>
/// </list>
/// Pure (no host dependencies) so it is unit-tested directly.
/// </summary>
public static class ProductionSecretsGuard
{
    public const string ConnectionStringKey = "ConnectionStrings:Default";
    public const string CertificatePassPhraseKey = "AuthServer:CertificatePassPhrase";
    public const string StringEncryptionPassPhraseKey = "StringEncryption:DefaultPassPhrase";
    public const string SmtpPasswordKey = "Email:SmtpPassword";
    public const string SmtpPasswordSettingKey = "Settings:Abp.Mailing.Smtp.Password";
    public const string S3SecretKey = "S3:SecretAccessKey";

    /// <summary>The committed (tracked) settings file that must never be the source of a production secret.</summary>
    public const string TrackedSettingsFileName = "appsettings.json";

    private const string TemplateCertificatePassPhrase = "3f027835-7c52-4bf7-bc1f-77582ca37aef";
    private const string TemplateStringEncryptionPassPhrase = "5pGzpfXjidNXJVDa";

    /// <summary>Database passwords that are public: the template's and the development ones in this repository's
    /// settings and compose file, and stock defaults.</summary>
    public static readonly string[] KnownDatabasePasswords = { "myPassword", "2025", "postgres", "password", "root" };

    private static readonly string[] SecretKeys =
    {
        ConnectionStringKey,
        CertificatePassPhraseKey,
        StringEncryptionPassPhraseKey
    };

    /// <summary>Secrets a deployment may leave unset; when set, they must not come from the tracked file.</summary>
    private static readonly string[] OptionalSecretKeys =
    {
        SmtpPasswordKey,
        SmtpPasswordSettingKey,
        S3SecretKey
    };

    /// <summary>Returns a human-readable problem per offending secret; empty when the configuration is safe.</summary>
    public static IReadOnlyList<string> FindProblems(IConfiguration configuration)
    {
        var problems = new List<string>();

        var databasePassword = FindDatabasePassword(configuration[ConnectionStringKey]);
        if (databasePassword != null && KnownDatabasePasswords.Contains(databasePassword, StringComparer.OrdinalIgnoreCase))
        {
            problems.Add($"{ConnectionStringKey} uses a well-known database password");
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

            foreach (var key in OptionalSecretKeys)
            {
                if (!string.IsNullOrEmpty(configuration[key]) && IsSuppliedByTrackedSettingsFile(root, key))
                {
                    problems.Add($"{key} is read from the tracked {TrackedSettingsFileName}");
                }
            }
        }

        return problems;
    }

    /// <summary>Problems of the OpenIddict signing/encryption certificate at <paramref name="path"/>: a localhost
    /// development certificate, one that cannot be opened with <paramref name="passphrase"/>, or one that has expired at
    /// <paramref name="now"/>. A missing file is reported by the host itself, so it is no problem here.</summary>
    public static IReadOnlyList<string> FindCertificateProblems(string path, string? passphrase, DateTimeOffset now)
    {
        var problems = new List<string>();
        if (!File.Exists(path))
        {
            return problems;
        }

        X509Certificate2 certificate;
        try
        {
            certificate = X509CertificateLoader.LoadPkcs12FromFile(path, passphrase);
        }
        catch (CryptographicException)
        {
            problems.Add($"{Path.GetFileName(path)} cannot be opened with {CertificatePassPhraseKey}");
            return problems;
        }

        using (certificate)
        {
            if (string.Equals(certificate.GetNameInfo(X509NameType.SimpleName, forIssuer: false), "localhost", StringComparison.OrdinalIgnoreCase))
            {
                problems.Add($"{Path.GetFileName(path)} is a localhost development certificate; provision a real one");
            }

            if (certificate.NotAfter.ToUniversalTime() <= now.UtcDateTime)
            {
                problems.Add($"{Path.GetFileName(path)} expired on {certificate.NotAfter.ToUniversalTime():yyyy-MM-dd}");
            }
        }

        return problems;
    }

    /// <summary>The password of a connection string (<c>Password</c> or <c>Pwd</c>), or null when it has none or cannot
    /// be read.</summary>
    public static string? FindDatabasePassword(string? connectionString)
    {
        if (string.IsNullOrWhiteSpace(connectionString))
        {
            return null;
        }

        try
        {
            var builder = new DbConnectionStringBuilder { ConnectionString = connectionString };
            foreach (var key in new[] { "Password", "Pwd" })
            {
                if (builder.TryGetValue(key, out var value) && value is string password && password.Length > 0)
                {
                    return password;
                }
            }
        }
        catch (ArgumentException)
        {
            // Not a key=value connection string: nothing to check.
        }

        return null;
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
