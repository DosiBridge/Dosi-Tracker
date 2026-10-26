using System;
using Dosi.Tracker.AiAssist.Provider;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>AiProviderConfigValidator: when this server may call its AI provider. Every problem turns AI off with a
/// state that names it; https only (plain http for a local server in Development); hosts from the allowlist only.</summary>
public class AiProviderConfigValidatorTests
{
    private static AiOptions Configured(Action<AiOptions>? change = null)
    {
        var options = new AiOptions { Enabled = true, OpenAI = { Model = "gpt-5.4-mini" } };
        change?.Invoke(options);
        return options;
    }

    private static AiProviderState Evaluate(AiOptions options, bool apiKeyPresent = true, bool isDevelopment = false) =>
        AiProviderConfigValidator.Evaluate(options, apiKeyPresent, isDevelopment);

    [Fact]
    public void The_Defaults_Are_Off()
    {
        Evaluate(new AiOptions(), apiKeyPresent: true).ShouldBe(AiProviderState.Disabled);
    }

    [Fact]
    public void OpenAi_With_A_Key_And_A_Model_Is_Configured()
    {
        Evaluate(Configured()).ShouldBe(AiProviderState.Configured);
        AiProviderConfigValidator.GetEndpoint(Configured()).ShouldBeNull();
    }

    [Fact]
    public void Off_Wins_Over_Every_Other_Problem()
    {
        Evaluate(Configured(o => { o.Enabled = false; o.Provider = "Nope"; o.OpenAI.Model = ""; }), apiKeyPresent: false)
            .ShouldBe(AiProviderState.Disabled);
    }

    [Theory]
    [InlineData("Gemini")]
    [InlineData("")]
    [InlineData(null)]
    public void An_Unknown_Provider_Is_Unsupported(string? provider)
    {
        Evaluate(Configured(o => o.Provider = provider!)).ShouldBe(AiProviderState.UnsupportedProvider);
    }

    [Theory]
    [InlineData("openai", "OpenAI")]
    [InlineData(" AzureOpenAI ", "AzureOpenAI")]
    [InlineData("openaicompatible", "OpenAICompatible")]
    public void Provider_Names_Are_Case_Insensitive(string provider, string canonical)
    {
        AiProviderConfigValidator.NormalizeProvider(provider).ShouldBe(canonical);
    }

    [Fact]
    public void A_Missing_Key_Or_Model_Is_Named()
    {
        Evaluate(Configured(), apiKeyPresent: false).ShouldBe(AiProviderState.MissingApiKey);
        Evaluate(Configured(o => o.OpenAI.Model = "  ")).ShouldBe(AiProviderState.MissingModel);
    }

    [Theory]
    [InlineData("AzureOpenAI")]
    [InlineData("OpenAICompatible")]
    public void Azure_And_Compatible_Servers_Need_A_Base_Url(string provider)
    {
        Evaluate(Configured(o => o.Provider = provider)).ShouldBe(AiProviderState.InvalidEndpoint);
        Evaluate(Configured(o =>
        {
            o.Provider = provider;
            o.OpenAI.BaseUrl = "https://example-resource.openai.azure.com/openai/v1/";
            o.OpenAI.AllowedHosts = "example-resource.openai.azure.com";
        })).ShouldBe(AiProviderState.Configured);
    }

    [Theory]
    [InlineData("https://api.openai.com/v1", true)]
    [InlineData("https://API.OpenAI.com./v1/", true)]
    [InlineData("http://api.openai.com/v1", false)]
    [InlineData("ftp://api.openai.com/v1", false)]
    [InlineData("api.openai.com/v1", false)]
    [InlineData("https://user:pass@api.openai.com/v1", false)]
    [InlineData("https://api.openai.com/v1?key=x", false)]
    [InlineData("https://api.openai.com/v1#x", false)]
    [InlineData("https://evil.example.com/v1", false)]
    [InlineData("https://api.openai.com.evil.example.com/v1", false)]
    public void The_Base_Url_Must_Be_Https_On_An_Allowed_Host(string baseUrl, bool configured)
    {
        Evaluate(Configured(o => o.OpenAI.BaseUrl = baseUrl))
            .ShouldBe(configured ? AiProviderState.Configured : AiProviderState.InvalidEndpoint);
    }

    [Theory]
    [InlineData("http://localhost:11434/v1", "localhost", true, true)]
    [InlineData("http://127.0.0.1:8000/v1", "127.0.0.1", true, true)]
    [InlineData("http://localhost:11434/v1", "localhost", false, false)]
    [InlineData("http://192.168.1.20:8000/v1", "192.168.1.20", true, false)]
    [InlineData("https://localhost:11434/v1", "localhost", false, true)]
    public void Plain_Http_Is_Allowed_Only_For_A_Local_Server_In_Development(
        string baseUrl, string allowedHost, bool isDevelopment, bool configured)
    {
        var options = Configured(o =>
        {
            o.Provider = AiOptions.ProviderOpenAiCompatible;
            o.OpenAI.BaseUrl = baseUrl;
            o.OpenAI.AllowedHosts = allowedHost;
        });

        Evaluate(options, isDevelopment: isDevelopment)
            .ShouldBe(configured ? AiProviderState.Configured : AiProviderState.InvalidEndpoint);
    }

    [Fact]
    public void A_Local_Server_Must_Still_Be_Allowlisted()
    {
        Evaluate(Configured(o => o.OpenAI.BaseUrl = "http://localhost:11434/v1"), isDevelopment: true)
            .ShouldBe(AiProviderState.InvalidEndpoint);
    }

    [Fact]
    public void The_Allowlist_Is_A_Trimmed_Case_Insensitive_Comma_List()
    {
        AiProviderConfigValidator.ParseAllowedHosts(" API.openai.com , example.com.,, example.com ")
            .ShouldBe(new[] { "api.openai.com", "example.com" });
        AiProviderConfigValidator.ParseAllowedHosts(null).ShouldBeEmpty();

        Evaluate(Configured(o =>
        {
            o.OpenAI.BaseUrl = "https://gateway.example.com/v1";
            o.OpenAI.AllowedHosts = "api.openai.com, Gateway.Example.com";
        })).ShouldBe(AiProviderState.Configured);
        Evaluate(Configured(o =>
        {
            o.OpenAI.BaseUrl = "https://api.openai.com/v1";
            o.OpenAI.AllowedHosts = "";
        })).ShouldBe(AiProviderState.InvalidEndpoint);
    }

    [Fact]
    public void The_Endpoint_Is_The_Trimmed_Base_Url()
    {
        AiProviderConfigValidator.GetEndpoint(Configured(o => o.OpenAI.BaseUrl = " https://api.openai.com/v1/ "))
            .ShouldBe(new Uri("https://api.openai.com/v1/"));
    }

    [Theory]
    [InlineData(30, 2, 30, 2)]
    [InlineData(0, -1, 1, 0)]
    [InlineData(10_000, 50, 300, 5)]
    public void Timeout_And_Retries_Are_Kept_In_Bounds(int timeoutSeconds, int retries, int expectedSeconds, int expectedRetries)
    {
        var options = Configured(o =>
        {
            o.RequestTimeoutSeconds = timeoutSeconds;
            o.MaxRetries = retries;
        });

        AiProviderConfigValidator.GetNetworkTimeout(options).ShouldBe(TimeSpan.FromSeconds(expectedSeconds));
        AiProviderConfigValidator.GetMaxRetries(options).ShouldBe(expectedRetries);
    }

    [Fact]
    public void The_Descriptor_Names_The_Provider_And_Model_Only()
    {
        var descriptor = AiProviderConfigValidator.Describe(Configured(o =>
        {
            o.Provider = "azureopenai";
            o.OpenAI.Model = " my-deployment ";
            o.OpenAI.BaseUrl = "https://example-resource.openai.azure.com/openai/v1/";
            o.Disclosure.ProviderDisplayName = "Azure OpenAI (EU)";
            o.StructuredOutput = "jsonobject";
            o.SupportsTools = false;
        }));

        descriptor.ShouldBe(new AiProviderDescriptor("Azure OpenAI (EU)", "AzureOpenAI", "my-deployment",
            SupportsJsonSchema: false, SupportsTools: false));
        descriptor.ToString().ShouldNotContain("example-resource");

        var plain = AiProviderConfigValidator.Describe(Configured(o => o.Disclosure.ProviderDisplayName = " "));
        plain.DisplayName.ShouldBe("OpenAI");
        plain.SupportsJsonSchema.ShouldBeTrue();
        plain.SupportsTools.ShouldBeTrue();
    }
}
