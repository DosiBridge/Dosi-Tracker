using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.HttpOverrides;
using Microsoft.Extensions.Configuration;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Security;

public class TrustedProxiesTests
{
    private static ForwardedHeadersOptions Configure(Dictionary<string, string?> settings)
    {
        var options = new ForwardedHeadersOptions();
        TrustedProxies.Configure(options, new ConfigurationBuilder().AddInMemoryCollection(settings).Build());
        return options;
    }

    [Fact]
    public void Without_Configuration_Only_Loopback_Is_Trusted_For_Both_Headers()
    {
        var options = Configure(new Dictionary<string, string?>());

        options.ForwardedHeaders.ShouldBe(ForwardedHeaders.XForwardedFor | ForwardedHeaders.XForwardedProto);
        options.KnownProxies.ShouldAllBe(address => IPAddress.IsLoopback(address));
        options.KnownIPNetworks.ShouldAllBe(network => IPAddress.IsLoopback(network.BaseAddress));
        options.ForwardLimit.ShouldBe(1);
    }

    [Fact]
    public void Proxies_And_Networks_Can_Be_Given_As_Comma_Separated_Strings()
    {
        var options = Configure(new Dictionary<string, string?>
        {
            ["App:ForwardedHeaders:KnownProxies"] = "10.0.0.5, 2001:db8::7",
            ["App:ForwardedHeaders:KnownNetworks"] = "10.1.0.0/16;172.16.0.0/12",
            ["App:ForwardedHeaders:ForwardLimit"] = "2"
        });

        options.KnownProxies.ShouldContain(IPAddress.Parse("10.0.0.5"));
        options.KnownProxies.ShouldContain(IPAddress.Parse("2001:db8::7"));
        options.KnownIPNetworks.ShouldContain(System.Net.IPNetwork.Parse("10.1.0.0/16"));
        options.KnownIPNetworks.ShouldContain(System.Net.IPNetwork.Parse("172.16.0.0/12"));
        options.ForwardLimit.ShouldBe(2);
    }

    [Fact]
    public void Proxies_And_Networks_Can_Be_Given_As_Arrays()
    {
        // JSON arrays and indexed environment variables (App__ForwardedHeaders__KnownNetworks__0) look like this.
        var options = Configure(new Dictionary<string, string?>
        {
            ["App:ForwardedHeaders:KnownProxies:0"] = "10.0.0.5",
            ["App:ForwardedHeaders:KnownProxies:1"] = "10.0.0.6",
            ["App:ForwardedHeaders:KnownNetworks:0"] = "10.0.0.0/8"
        });

        options.KnownProxies.Count(address => !IPAddress.IsLoopback(address)).ShouldBe(2);
        options.KnownIPNetworks.ShouldContain(System.Net.IPNetwork.Parse("10.0.0.0/8"));
    }

    [Theory]
    [InlineData("App:ForwardedHeaders:KnownProxies", "proxy.internal")]
    [InlineData("App:ForwardedHeaders:KnownNetworks", "10.0.0.0")]
    [InlineData("App:ForwardedHeaders:KnownNetworks", "10.0.0.0/33")]
    [InlineData("App:ForwardedHeaders:ForwardLimit", "0")]
    public void A_Misconfigured_Entry_Should_Fail_At_Start_Up(string key, string value)
    {
        Should.Throw<InvalidOperationException>(() => Configure(new Dictionary<string, string?> { [key] = value }))
            .Message.ShouldContain(key);
    }

    [Theory]
    [InlineData(null, null, false)]
    [InlineData("10.0.0.5", null, true)]
    [InlineData(null, "10.0.0.0/8", true)]
    [InlineData(" ", " ", false)]
    public void Configured_Proxies_Are_Detected_So_The_Host_Can_Warn_Without_Them(string? proxies, string? networks, bool expected)
    {
        var configuration = new ConfigurationBuilder().AddInMemoryCollection(new Dictionary<string, string?>
        {
            ["App:ForwardedHeaders:KnownProxies"] = proxies,
            ["App:ForwardedHeaders:KnownNetworks"] = networks
        }).Build();

        TrustedProxies.HasConfiguredProxies(configuration).ShouldBe(expected);
    }
}
