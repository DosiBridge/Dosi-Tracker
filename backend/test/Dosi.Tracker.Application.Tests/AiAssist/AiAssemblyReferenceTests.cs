using System;
using System.Linq;
using System.Reflection;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.AiAssist;

/// <summary>Provider SDKs stay in the API host: the domain, contracts and application assemblies know only
/// Microsoft.Extensions.AI's abstractions, so swapping the provider never touches feature code and no test of them
/// can construct a real provider client.</summary>
public class AiAssemblyReferenceTests
{
    private static readonly string[] ProviderAssemblies = { "OpenAI", "Microsoft.Extensions.AI.OpenAI", "Azure.AI.OpenAI" };

    public static TheoryData<string> Layers() => new()
    {
        typeof(TrackerDomainSharedModule).Assembly.GetName().Name!,
        typeof(TrackerDomainModule).Assembly.GetName().Name!,
        typeof(TrackerApplicationContractsModule).Assembly.GetName().Name!,
        typeof(TrackerApplicationModule).Assembly.GetName().Name!
    };

    [Theory]
    [MemberData(nameof(Layers))]
    public void No_Layer_References_A_Provider_Sdk(string assemblyName)
    {
        var references = Assembly.Load(assemblyName).GetReferencedAssemblies().Select(a => a.Name).ToList();

        references.Where(name => ProviderAssemblies.Contains(name, StringComparer.OrdinalIgnoreCase)).ShouldBeEmpty();
    }

    [Fact]
    public void The_Application_Uses_The_Abstractions()
    {
        typeof(TrackerApplicationModule).Assembly.GetReferencedAssemblies().Select(a => a.Name)
            .ShouldContain("Microsoft.Extensions.AI.Abstractions");
    }
}
