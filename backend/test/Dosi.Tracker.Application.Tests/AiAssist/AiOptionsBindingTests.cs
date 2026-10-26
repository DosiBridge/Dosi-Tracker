using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using Microsoft.Extensions.Configuration;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>AiOptions (section "AI") as operators set it: from environment variables, whose "__" becomes ":" (as the
/// environment-variables configuration provider does), with case-insensitive feature and plan keys. The options graph
/// has no place for a key.</summary>
public class AiOptionsBindingTests
{
    private static AiOptions Bind(params (string EnvironmentVariable, string Value)[] variables)
    {
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(variables.ToDictionary(v => v.EnvironmentVariable.Replace("__", ":"), v => (string?)v.Value))
            .Build();

        var options = new AiOptions();
        configuration.GetSection(AiOptions.SectionName).Bind(options);
        return options;
    }

    [Fact]
    public void The_Defaults_Are_Off_And_Match_The_Spec()
    {
        var options = Bind();

        options.Enabled.ShouldBeFalse();
        options.Provider.ShouldBe("OpenAI");
        options.OpenAI.Model.ShouldBe("");
        options.OpenAI.AllowedHosts.ShouldBe("api.openai.com");
        options.RequestTimeoutSeconds.ShouldBe(30);
        options.MaxRetries.ShouldBe(2);
        options.Temperature.ShouldBeNull();
        options.ReasoningEffort.ShouldBeNull();
        options.StructuredOutput.ShouldBe(AiOptions.StructuredOutputJsonSchema);
        options.SupportsTools.ShouldBeTrue();
        options.RepairAttempts.ShouldBe(1);
        options.Assistant.MaxToolRounds.ShouldBe(3);
        options.Assistant.TurnTimeoutSeconds.ShouldBe(45);
        options.Concurrency.Global.ShouldBe(16);
        options.Concurrency.PerWorkspace.ShouldBe(3);
        options.Budget.MonthlyTokenLimit.ShouldBe(20_000_000);
        options.Budget.MonthlyUsdLimit.ShouldBeNull();
        options.Disclosure.ProviderDisplayName.ShouldBe("OpenAI");
        options.Disclosure.Version.ShouldBe(0);
        options.Usage.RetentionDays.ShouldBe(400);
        options.Pricing.ShouldBeEmpty();
        options.Features.ShouldBeEmpty();
    }

    [Fact]
    public void Environment_Style_Keys_Bind()
    {
        var options = Bind(
            ("AI__Enabled", "true"),
            ("AI__Provider", "AzureOpenAI"),
            ("AI__OpenAI__Model", "gpt-5.4-mini"),
            ("AI__ReasoningEffort", "low"),
            ("AI__Temperature", "0.2"),
            ("AI__Features__MemberDaySummary__Enabled", "false"),
            ("AI__Features__TaskDrafts__Model", "gpt-4.1-mini"),
            ("AI__Features__TaskDrafts__MaxOutputTokens", "900"),
            ("AI__Pricing__0__Model", "gpt-5.4-mini"),
            ("AI__Pricing__0__InputPer1M", "0.25"),
            ("AI__Pricing__0__CachedInputPer1M", "0.025"),
            ("AI__Pricing__0__OutputPer1M", "2"),
            ("AI__Pricing__1__Model", "gpt-4.1-mini"),
            ("AI__Quotas__Default__UserPerMinute", "9"),
            ("AI__Disclosure__Version", "2"));

        options.Enabled.ShouldBeTrue();
        options.Provider.ShouldBe("AzureOpenAI");
        options.OpenAI.Model.ShouldBe("gpt-5.4-mini");
        options.ReasoningEffort.ShouldBe("low");
        options.Temperature.ShouldBe(0.2f);
        options.Features["MemberDaySummary"].Enabled.ShouldBeFalse();
        options.Pricing.Count.ShouldBe(2);
        options.Pricing[0].Model.ShouldBe("gpt-5.4-mini");
        options.Pricing[0].InputPer1M.ShouldBe(0.25m);
        options.Pricing[0].CachedInputPer1M.ShouldBe(0.025m);
        options.Pricing[0].OutputPer1M.ShouldBe(2m);
        options.Pricing[1].CachedInputPer1M.ShouldBeNull();
        options.Quotas.Default.UserPerMinute.ShouldBe(9);
        options.Quotas.Default.MonthlyRequests.ShouldBe(200);
        options.Disclosure.Version.ShouldBe(2);

        options.GetFeature(AiFeatureIds.MemberDaySummary).Enabled.ShouldBeFalse();
        options.GetFeature(AiFeatureIds.PeriodSummary).Enabled.ShouldBeTrue();
        options.GetModel(AiFeatureIds.TaskDrafts).ShouldBe("gpt-4.1-mini");
        options.GetModel(AiFeatureIds.Ask).ShouldBe("gpt-5.4-mini");
        options.GetMaxOutputTokens(AiFeatureIds.TaskDrafts).ShouldBe(900);
        options.GetMaxOutputTokens(AiFeatureIds.NoteSuggestions).ShouldBe(200);
    }

    [Fact]
    public void Feature_And_Plan_Keys_Are_Case_Insensitive()
    {
        var options = Bind(
            ("AI__Features__memberdaysummary__Enabled", "false"),
            ("AI__Quotas__Plans__starter__MonthlyRequests", "750"),
            ("AI__Quotas__Plans__Enterprise__MonthlyRequests", "9000"));

        options.GetFeature(AiFeatureIds.MemberDaySummary).Enabled.ShouldBeFalse();
        options.Quotas.For("Starter").MonthlyRequests.ShouldBe(750);
        options.Quotas.For("STARTER").UserDailyRequests.ShouldBe(30);
        options.Quotas.For("enterprise").MonthlyRequests.ShouldBe(9000);
    }

    [Fact]
    public void Plans_Have_The_Spec_Allowances_And_Unknown_Plans_Fall_Back_To_Default()
    {
        var quotas = Bind().Quotas;

        quotas.For("Free").MonthlyRequests.ShouldBe(50);
        quotas.For("free").MonthlyTokens.ShouldBe(200_000);
        quotas.For("Business").UserPerMinute.ShouldBe(6);
        quotas.For("Platinum").ShouldBeSameAs(quotas.Default);
        quotas.For(null).ShouldBeSameAs(quotas.Default);
        quotas.Default.UserDailyRequests.ShouldBe(20);
    }

    [Fact]
    public void A_Blank_Feature_Model_Falls_Back_And_No_Model_Is_Null()
    {
        var options = Bind(("AI__Features__Ask__Model", " "), ("AI__Features__Ask__MaxOutputTokens", "0"));

        options.GetModel(AiFeatureIds.Ask).ShouldBeNull();
        options.GetMaxOutputTokens(AiFeatureIds.Ask).ShouldBe(700);
    }

    [Fact]
    public void A_Configured_Key_Lands_Nowhere_In_The_Options()
    {
        // Reflection over the whole options graph: no property could ever hold a key, so none can be serialized or logged.
        var offenders = new List<string>();
        Walk(typeof(AiOptions), nameof(AiOptions), new HashSet<Type>(), offenders);

        offenders.ShouldBeEmpty();
        Bind(("AI__OpenAI__ApiKey", "test-not-a-key")).OpenAI.ShouldNotBeNull();
    }

    private static void Walk(Type type, string path, HashSet<Type> seen, List<string> offenders)
    {
        if (!seen.Add(type))
        {
            return;
        }

        foreach (var property in type.GetProperties(BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance))
        {
            var name = property.Name;
            if (name.Contains("ApiKey", StringComparison.OrdinalIgnoreCase) ||
                name.Contains("Secret", StringComparison.OrdinalIgnoreCase) ||
                name.Contains("Password", StringComparison.OrdinalIgnoreCase) ||
                name.Equals("Key", StringComparison.OrdinalIgnoreCase))
            {
                offenders.Add($"{path}.{name}");
            }

            foreach (var candidate in new[] { property.PropertyType }.Concat(property.PropertyType.GetGenericArguments()))
            {
                if (candidate.Namespace?.StartsWith("Dosi.Tracker", StringComparison.Ordinal) == true)
                {
                    Walk(candidate, $"{path}.{name}", seen, offenders);
                }
            }
        }
    }
}
