using System;
using System.Collections.Generic;
using Microsoft.Extensions.Configuration;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

/// <summary>App:CorsOrigins: absolute origins only (a wildcard first label allowed), a trailing slash dropped, duplicates
/// dropped, and never "*" — the API admits credentialed requests.</summary>
public class CorsOriginsTests
{
    [Fact]
    public void Origins_Are_Trimmed_And_Deduplicated()
    {
        CorsOrigins.Parse(" https://app.example.com/ , http://localhost:6005,https://APP.example.com ,https://*.example.com")
            .ShouldBe(new[] { "https://app.example.com", "http://localhost:6005", "https://*.example.com" });
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData(" , ")]
    public void No_Configured_Origin_Admits_No_Browser(string? value)
    {
        CorsOrigins.Parse(value).ShouldBeEmpty();
    }

    [Theory]
    [InlineData("*")]
    [InlineData("https://*")]
    [InlineData("https://app.*.example.com")]
    [InlineData("https://**.example.com")]
    [InlineData("https://*example.com")]
    [InlineData("app.example.com")]
    [InlineData("ftp://app.example.com")]
    [InlineData("https://app.example.com/dashboard")]
    [InlineData("https://app.example.com?x=1")]
    [InlineData("https://user@app.example.com")]
    public void An_Unsafe_Or_Malformed_Origin_Stops_The_Start_Up(string origin)
    {
        Should.Throw<InvalidOperationException>(() => CorsOrigins.Parse("https://ok.example.com," + origin))
            .Message.ShouldContain(CorsOrigins.Key);
    }

    [Fact]
    public void The_Origins_Are_Read_From_App_CorsOrigins()
    {
        var configuration = new ConfigurationBuilder()
            .AddInMemoryCollection(new Dictionary<string, string?> { [CorsOrigins.Key] = "https://app.example.com:8443" })
            .Build();

        CorsOrigins.Parse(configuration).ShouldBe(new[] { "https://app.example.com:8443" });
    }
}
