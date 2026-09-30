using System;
using System.Collections.Generic;
using System.IO;
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
        problems.ShouldContain(p => p.Contains("template database password"));
        problems.ShouldContain(p => p.Contains(ProductionSecretsGuard.CertificatePassPhraseKey) && p.Contains("template default"));
        problems.ShouldContain(p => p.Contains(ProductionSecretsGuard.StringEncryptionPassPhraseKey) && p.Contains("template default"));
    }

    [Fact]
    public void A_Real_Looking_Secret_Committed_To_The_Tracked_File_Is_Still_Rejected()
    {
        // e.g. a developer's real local password committed by mistake — not a template value, but tracked.
        var config = Build(Tracked("Host=db;User ID=postgres;Password=2025;", "some-real-cert-pass", "some-real-key"));

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
}
