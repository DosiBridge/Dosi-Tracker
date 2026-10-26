using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.AiAssist.Access;
using Dosi.Tracker.AiAssist.Context;
using Dosi.Tracker.AiAssist.Fakes;
using Dosi.Tracker.AiAssist.Features.Facts;
using Dosi.Tracker.AiAssist.Pipeline;
using Dosi.Tracker.AiAssist.Read;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;
using Shouldly;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>
/// The AI cards end to end, as the app service serves them, over the read-layer workspace
/// (<see cref="AiReadLayerTestBase{TStartupModule}"/>: Ayesha Rahman, AYESHA-LAPTOP, 98765 keystrokes, a salary window
/// title and a note): helpers to script the model, build a feature's bundle directly, and check what was sent.
/// </summary>
public abstract class AiFeatureTestBase<TStartupModule> : AiReadLayerTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static readonly Regex GuidPattern = new("[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}");

    protected CapturingLoggerProvider Logs { get; } = new();

    protected AiFeatureTestBase()
    {
        // Several requests a minute; the burst limit has tests of its own.
        AiOptions.Quotas.Default.UserPerMinute = 100;
    }

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.AddSingleton<ILoggerProvider>(Logs);
    }

    protected FakeChatClient Model => Provider.Client;

    protected IAiAssistAppService Ai => GetRequiredService<IAiAssistAppService>();

    /// <summary>The seeded workspace with AI assist on (and the given policy).</summary>
    protected async Task ReadyAsync(bool titles = false, bool notes = false)
    {
        await SeedAsync();
        await SetPolicyAsync(titles: titles, notes: notes);
    }

    protected static string DayText => Day.ToString("yyyy-MM-dd");

    /// <summary>A feature's bundle for <paramref name="input"/>, built as the runner would (authorize, then read).</summary>
    protected Task<(AiAudienceScope Scope, AiContextBundle Bundle)> BundleAsync<TIn, TRaw, TOut>(
        IAiInsightFeature<TIn, TRaw, TOut> feature, TIn input, string zone = "UTC")
        where TRaw : class
    {
        return WithUnitOfWorkAsync(async () =>
        {
            var caller = await GetRequiredService<AiCallerContextFactory>().CreateAsync(zone);
            var scope = await feature.AuthorizeAsync(caller, input);
            var bundle = await feature.BuildBundleAsync(caller, input, scope);
            return (scope, bundle);
        });
    }

    /// <summary>A new instance of a feature (each request gets its own).</summary>
    protected T Feature<T>()
        where T : notnull => GetRequiredService<T>();

    /// <summary>The display of the fact <c>lists.totals[key]</c>.</summary>
    protected static string Total(AiContextBundle bundle, string key)
    {
        var id = AiFeatureOutput.ListValue(bundle, AiPeriodFacts.TotalsList, key).ShouldNotBeNull(key);
        return bundle.Facts[id].Display;
    }

    /// <summary>The display of the fact an item of a list names under <paramref name="key"/>.</summary>
    protected static string FactOf(AiContextBundle bundle, System.Text.Json.Nodes.JsonObject item, string key) =>
        bundle.Facts[AiFeatureOutput.Value(item, key).ShouldNotBeNull(key)].Display;

    /// <summary>What was sent must never hold a name, an email, a device name, an OS version, an input count, an id or a
    /// capture.</summary>
    protected static void ShouldHoldNothingPrivate(FakeChatCall call)
    {
        var text = call.AllText;
        text.ShouldNotContain(AyeshaName);
        text.ShouldNotContain("Ayesha");
        text.ShouldNotContain("@acme.test");
        text.ShouldNotContain("Caller Person");
        text.ShouldNotContain(SecretDeviceName);
        text.ShouldNotContain(SecretOsVersion);
        text.ShouldNotContain(SecretKeystrokes.ToString());
        text.ShouldNotContain(SecretClicks.ToString());
        text.ShouldNotContain("data:image");
        text.ShouldNotContain("Secret task description");
        GuidPattern.IsMatch(text).ShouldBeFalse("no id of any kind (blocks, captures, members, projects) is sent");
    }

    protected Task<List<AiUsageRecord>> RowsAsync() =>
        WithUnitOfWorkAsync(async () => (await GetRequiredService<IRepository<AiUsageRecord, Guid>>().GetListAsync())
            .OrderBy(r => r.CreationTime)
            .ToList());

    /// <summary>A block of <paramref name="userId"/> without input counts, titles or notes.</summary>
    protected async Task<Guid> InsertBlockAsync(
        Guid userId, Guid projectId, DateTime start, int minutes, int? clockSkew = null, int? injected = null,
        int? productivity = 60, string? platform = "windows", string? agentVersion = "1.4.0", string? note = null)
    {
        var id = Guid.NewGuid();
        await WithUnitOfWorkAsync(async () =>
        {
            await GetRequiredService<IRepository<Activity, Guid>>().InsertAsync(
                new Activity(id, WorkspaceId, userId, projectId, Guid.NewGuid(), start, start.AddMinutes(minutes))
                {
                    Productivity = productivity,
                    ClockSkewSeconds = clockSkew,
                    InjectedInputEvents = injected,
                    Platform = platform,
                    AgentVersion = agentVersion,
                    DeviceId = Guid.NewGuid(),
                    Description = note,
                    EndReason = "interval",
                    ActiveWindowsJson = JsonSerializer.Serialize(new[] { new WindowInfoDto { AppName = "Code.exe", Seconds = minutes * 60 } })
                },
                autoSave: true);
        });
        return id;
    }

    /// <summary>The JSON of an answer (property names as the schemas spell them).</summary>
    protected static string Json(object answer) => JsonSerializer.Serialize(answer);

    protected static T Single<T>(IEnumerable<T> items) => items.ShouldHaveSingleItem();
}
