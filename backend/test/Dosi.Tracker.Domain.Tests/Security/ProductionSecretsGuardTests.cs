using System;
using System.Collections.Generic;
using System.IO;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using Microsoft.Extensions.Configuration;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

public sealed class ProductionSecretsGuardTests : IDisposable
{
    private const string TemplateConnection = "Host=localhost;Database=Tracker;User ID=root;Password=myPassword;";
    private const string TemplateCert = "3f027835-7c52-4bf7-bc1f-77582ca37aef";
    private const string TemplateEncryption = "5pGzpfXjidNXJVDa";

    private readonly string _dir = Path.Combine(Path.GetTempPath(), "dosi-guard-" + Guid.NewGuid().ToString("N"));

    public ProductionSecretsGuardTests()
    {
        Directory.CreateDirectory(_dir);
    }

    public void Dispose()
    {
        try { Directory.Delete(_dir, recursive: true); } catch (IOException) { /* best effort */ }
    }

    /// <summary>Mimics the host: tracked appsettings.json first, then later sources (env vars, secrets, …).</summary>
    private IConfigurationRoot Build(string trackedJson, Dictionary<string, string?>? laterOverrides = null)
    {
        File.WriteAllText(Path.Combine(_dir, "appsettings.json"), trackedJson);

        var builder = new ConfigurationBuilder()
            .SetBasePath(_dir)
            .AddJsonFile("appsettings.json", optional: false);

        if (laterOverrides != null)
        {
            builder.AddInMemoryCollection(laterOverrides);
        }

        return builder.Build();
    }

    private static string Tracked(string connection, string cert, string encryption) =>
        $$"""
        {
          "ConnectionStrings": { "Default": "{{connection}}" },
          "AuthServer": { "CertificatePassPhrase": "{{cert}}" },
          "StringEncryption": { "DefaultPassPhrase": "{{encryption}}" }
        }
        """;

    [Fact]
    public void Template_Values_From_The_Tracked_File_Are_All_Rejected()
    {
        var config = Build(Tracked(TemplateConnection, TemplateCert, TemplateEncryption));

        var problems = ProductionSecretsGuard.FindProblems(config);

        // 3 template-value problems + 3 "read from tracked appsettings.json" problems.
        problems.Count.ShouldBe(6);
        problems.ShouldContain(p => p.Contains("well-known database password"));
        problems.ShouldContain(p => p.Contains(ProductionSecretsGuard.CertificatePassPhraseKey) && p.Contains("template default"));
        problems.ShouldContain(p => p.Contains(ProductionSecretsGuard.StringEncryptionPassPhraseKey) && p.Contains("template default"));
    }

    [Fact]
    public void A_Real_Looking_Secret_Committed_To_The_Tracked_File_Is_Still_Rejected()
    {
        // e.g. a developer's real local password committed by mistake — not a template value, but tracked.
        var config = Build(Tracked("Host=db;User ID=postgres;Password=Xq7-local-only;", "some-real-cert-pass", "some-real-key"));

        var problems = ProductionSecretsGuard.FindProblems(config);

        problems.Count.ShouldBe(3);
        problems.ShouldAllBe(p => p.Contains("tracked appsettings.json"));
    }

    [Fact]
    public void Secrets_Overridden_By_A_Later_Provider_Are_Accepted()
    {
        var config = Build(
            Tracked(TemplateConnection, TemplateCert, TemplateEncryption),
            new Dictionary<string, string?>
            {
                [ProductionSecretsGuard.ConnectionStringKey] = "Host=db;User ID=tracker;Password=s3cr3t-from-env;",
                [ProductionSecretsGuard.CertificatePassPhraseKey] = "cert-pass-from-vault",
                [ProductionSecretsGuard.StringEncryptionPassPhraseKey] = "encryption-pass-from-vault"
            });

        ProductionSecretsGuard.FindProblems(config).ShouldBeEmpty();
    }

    [Fact]
    public void Overriding_With_The_Template_Value_Is_Still_Rejected()
    {
        var config = Build(
            Tracked("Host=db;Password=x;", "x", "x"),
            new Dictionary<string, string?>
            {
                [ProductionSecretsGuard.ConnectionStringKey] = "Host=db;Password=from-env;",
                [ProductionSecretsGuard.CertificatePassPhraseKey] = TemplateCert,
                [ProductionSecretsGuard.StringEncryptionPassPhraseKey] = "from-env"
            });

        var problems = ProductionSecretsGuard.FindProblems(config);

        problems.ShouldHaveSingleItem().ShouldContain("template default");
    }

    [Fact]
    public void Source_Detection_Picks_The_Effective_Last_Provider()
    {
        var config = Build(
            Tracked("tracked", "tracked", "tracked"),
            new Dictionary<string, string?> { [ProductionSecretsGuard.CertificatePassPhraseKey] = "override" });

        ProductionSecretsGuard.IsSuppliedByTrackedSettingsFile(config, ProductionSecretsGuard.ConnectionStringKey).ShouldBeTrue();
        ProductionSecretsGuard.IsSuppliedByTrackedSettingsFile(config, ProductionSecretsGuard.CertificatePassPhraseKey).ShouldBeFalse();
        ProductionSecretsGuard.IsSuppliedByTrackedSettingsFile(config, "Missing:Key").ShouldBeFalse();
    }

    [Fact]
    public void A_Differently_Named_Settings_File_Is_Not_Treated_As_Tracked()
    {
        File.WriteAllText(Path.Combine(_dir, "appsettings.Production.json"),
            Tracked("Host=db;Password=prod;", "prod-cert", "prod-key"));
        var config = new ConfigurationBuilder()
            .SetBasePath(_dir)
            .AddJsonFile("appsettings.Production.json", optional: false)
            .Build();

        ProductionSecretsGuard.FindProblems(config).ShouldBeEmpty();
    }

    [Theory]
    [InlineData("Host=db;User ID=tracker;Password=2025;")]
    [InlineData("Host=db;User ID=tracker;Password=myPassword;")]
    [InlineData("Host=db;User ID=postgres;Pwd=postgres;")]
    [InlineData("Host=db;Username=tracker;Password=PASSWORD;")]
    public void A_Published_Or_Stock_Database_Password_Is_Rejected_Even_From_The_Environment(string connectionString)
    {
        var config = Build(
            Tracked(TemplateConnection, TemplateCert, TemplateEncryption),
            new Dictionary<string, string?>
            {
                [ProductionSecretsGuard.ConnectionStringKey] = connectionString,
                [ProductionSecretsGuard.CertificatePassPhraseKey] = "cert-pass-from-vault",
                [ProductionSecretsGuard.StringEncryptionPassPhraseKey] = "encryption-pass-from-vault"
            });

        ProductionSecretsGuard.FindProblems(config).ShouldHaveSingleItem().ShouldContain("well-known database password");
    }

    [Theory]
    [InlineData("Host=db;Password=s3cr3t;", "s3cr3t")]
    [InlineData("Host=db;pwd=s3cr3t;", "s3cr3t")]
    [InlineData("Host=db;Integrated Security=true;", null)]
    [InlineData("not a connection string = = ;;", null)]
    [InlineData("", null)]
    public void The_Database_Password_Is_Read_From_The_Connection_String(string connectionString, string? expected)
    {
        ProductionSecretsGuard.FindDatabasePassword(connectionString).ShouldBe(expected);
    }

    [Fact]
    public void Smtp_And_S3_Secrets_Must_Not_Come_From_The_Tracked_File_When_They_Are_Set()
    {
        var tracked = $$"""
        {
          "ConnectionStrings": { "Default": "x" },
          "AuthServer": { "CertificatePassPhrase": "x" },
          "StringEncryption": { "DefaultPassPhrase": "x" },
          "Email": { "SmtpPassword": "smtp-secret-in-git" },
          "S3": { "SecretAccessKey": "" }
        }
        """;
        var overrides = new Dictionary<string, string?>
        {
            [ProductionSecretsGuard.ConnectionStringKey] = "Host=db;Password=from-env;",
            [ProductionSecretsGuard.CertificatePassPhraseKey] = "from-env",
            [ProductionSecretsGuard.StringEncryptionPassPhraseKey] = "from-env"
        };

        // An empty S3 secret in the tracked file is no secret; the SMTP password committed there is one.
        ProductionSecretsGuard.FindProblems(Build(tracked, overrides))
            .ShouldHaveSingleItem().ShouldContain(ProductionSecretsGuard.SmtpPasswordKey);

        overrides[ProductionSecretsGuard.SmtpPasswordKey] = "smtp-secret-from-env";
        ProductionSecretsGuard.FindProblems(Build(tracked, overrides)).ShouldBeEmpty();
    }

    /// <summary>A tracked file whose non-AI secrets are all overridden later, so only the AI key decides.</summary>
    private static Dictionary<string, string?> SafeOverrides() => new()
    {
        [ProductionSecretsGuard.ConnectionStringKey] = "Host=db;Password=from-env;",
        [ProductionSecretsGuard.CertificatePassPhraseKey] = "from-env",
        [ProductionSecretsGuard.StringEncryptionPassPhraseKey] = "from-env"
    };

    private static string TrackedWithAi(string aiOpenAiJson) =>
        $$"""
        {
          "ConnectionStrings": { "Default": "x" },
          "AuthServer": { "CertificatePassPhrase": "x" },
          "StringEncryption": { "DefaultPassPhrase": "x" },
          "AI": { "Enabled": true, "OpenAI": {{aiOpenAiJson}} }
        }
        """;

    [Fact]
    public void An_Ai_Key_Committed_To_The_Tracked_File_Is_Rejected_Without_Echoing_It()
    {
        const string fakeKey = "test-not-a-key";
        var config = Build(TrackedWithAi($$"""{ "ApiKey": "{{fakeKey}}", "Model": "fake-model" }"""), SafeOverrides());

        var problem = ProductionSecretsGuard.FindProblems(config).ShouldHaveSingleItem();

        problem.ShouldContain(ProductionSecretsGuard.AiApiKeyKey);
        problem.ShouldContain("tracked appsettings.json");
        problem.ShouldNotContain(fakeKey);
    }

    [Fact]
    public void An_Ai_Key_From_A_Later_Provider_Is_Accepted()
    {
        var overrides = SafeOverrides();
        overrides[ProductionSecretsGuard.AiApiKeyKey] = "test-not-a-key-from-env";

        // Also when the tracked file carries an (empty or placeholder) key that the environment overrides.
        ProductionSecretsGuard.FindProblems(Build(TrackedWithAi("""{ "Model": "fake-model" }"""), overrides)).ShouldBeEmpty();
        ProductionSecretsGuard.FindProblems(Build(TrackedWithAi("""{ "ApiKey": "test-not-a-key" }"""), overrides)).ShouldBeEmpty();
    }

    [Fact]
    public void An_Absent_Or_Empty_Ai_Key_Is_No_Problem()
    {
        ProductionSecretsGuard.FindProblems(Build(TrackedWithAi("""{ "Model": "" }"""), SafeOverrides())).ShouldBeEmpty();
        ProductionSecretsGuard.FindProblems(Build(TrackedWithAi("""{ "ApiKey": "" }"""), SafeOverrides())).ShouldBeEmpty();
    }

    private string WriteCertificate(string subject, DateTimeOffset notBefore, DateTimeOffset notAfter, string passphrase)
    {
        using var key = RSA.Create(2048);
        var request = new CertificateRequest(subject, key, HashAlgorithmName.SHA256, RSASignaturePadding.Pkcs1);
        using var certificate = request.CreateSelfSigned(notBefore, notAfter);
        var path = Path.Combine(_dir, Guid.NewGuid().ToString("N") + ".pfx");
        File.WriteAllBytes(path, certificate.Export(X509ContentType.Pfx, passphrase));
        return path;
    }

    [Fact]
    public void A_Localhost_Development_Certificate_Is_Rejected_And_A_Real_One_Accepted()
    {
        var now = DateTimeOffset.UtcNow;
        var development = WriteCertificate("CN=localhost", now.AddDays(-1), now.AddYears(1), "pass");
        var real = WriteCertificate("CN=tracker.example.com", now.AddDays(-1), now.AddYears(1), "pass");

        ProductionSecretsGuard.FindCertificateProblems(development, "pass", now)
            .ShouldHaveSingleItem().ShouldContain("localhost development certificate");
        ProductionSecretsGuard.FindCertificateProblems(real, "pass", now).ShouldBeEmpty();
    }

    [Fact]
    public void An_Expired_Or_Unopenable_Certificate_Is_Rejected_And_A_Missing_One_Left_To_The_Host()
    {
        var now = DateTimeOffset.UtcNow;
        var expired = WriteCertificate("CN=tracker.example.com", now.AddYears(-2), now.AddDays(-1), "pass");
        var valid = WriteCertificate("CN=tracker.example.com", now.AddDays(-1), now.AddYears(1), "pass");

        ProductionSecretsGuard.FindCertificateProblems(expired, "pass", now).ShouldHaveSingleItem().ShouldContain("expired");
        ProductionSecretsGuard.FindCertificateProblems(valid, "wrong-pass", now)
            .ShouldHaveSingleItem().ShouldContain(ProductionSecretsGuard.CertificatePassPhraseKey);
        ProductionSecretsGuard.FindCertificateProblems(Path.Combine(_dir, "missing.pfx"), "pass", now).ShouldBeEmpty();
    }
}
