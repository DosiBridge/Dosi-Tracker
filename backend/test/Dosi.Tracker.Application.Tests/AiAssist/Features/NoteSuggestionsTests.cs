using System;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Activities;
using Dosi.Tracker.AiAssist.Provider;
using Dosi.Tracker.Permissions;
using Dosi.Tracker.Security;
using Dosi.Tracker.Workspaces;
using Microsoft.Extensions.DependencyInjection;
using Shouldly;
using Volo.Abp.Authorization;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Modularity;
using Xunit;

namespace Dosi.Tracker.AiAssist.Features;

/// <summary>Note suggestions: offered only to someone who could save the note, never saved by AI.</summary>
public abstract class NoteSuggestionsTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    private static string Answer(params (string Text, string Basis)[] suggestions) =>
        Json(new { suggestions = suggestions.Select(s => new { text = s.Text, basis = s.Basis }).ToArray() });

    private Task<string?> NoteOfAsync(Guid activityId) =>
        WithUnitOfWorkAsync(async () => (await GetRequiredService<IRepository<Activity, Guid>>().GetAsync(activityId)).Description);

    [Fact]
    public async Task The_Owner_Of_A_Recent_Block_Should_Get_Suggestions_And_The_Block_Should_Stay_Unchanged()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer(("Reviewed the {P1} pages in {A1}", "apps"), ("Updated {P1}", "project")));

        var result = await Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = CallerBlockId });

        result.Meta.Status.ShouldBe(AiResultStatuses.Generated);
        result.Suggestions.Select(s => (s.Text, s.Basis)).ShouldBe(new[]
        {
            ("Reviewed the Website pages in Excel", "apps"), ("Updated Website", "project")
        });
        (await NoteOfAsync(CallerBlockId)).ShouldBe(SecretNote);
        var call = Model.Calls.ShouldHaveSingleItem();
        ShouldHoldNothingPrivate(call);
        call.AllText.ShouldContain("30 min");
        (await RowsAsync()).ShouldHaveSingleItem().Audience.ShouldBe(AiUsageRecord.SelfAudience);
    }

    [Fact]
    public async Task Window_Titles_And_Notes_Should_Be_Sent_Only_When_The_Workspace_Allows_Them()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer(("Worked on {P1}", "project")));
        await Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = CallerBlockId });
        Model.Calls.Last().AllText.ShouldNotContain(SalaryTitle);
        Model.Calls.Last().AllText.ShouldNotContain(SecretNote);

        await SetPolicyAsync(titles: true, notes: true);
        Model.EnqueueJson(Answer(("Reviewed the Q3 sheet in {A1}", "window-titles")));
        var result = await Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = CallerBlockId });

        var call = Model.Calls.Last();
        call.AllText.ShouldContain(SalaryTitle);
        call.AllText.ShouldContain(SecretNote);
        // "3" is in a quoted title, so a creative answer may repeat it.
        result.Suggestions.ShouldHaveSingleItem().Text.ShouldBe("Reviewed the Q3 sheet in Excel");
        result.Meta.DataCategories.ShouldContain(AiDataCategories.WindowTitles);
        result.Meta.DataCategories.ShouldContain(AiDataCategories.TaskNotes);
    }

    [Fact]
    public async Task People_Unquoted_Numbers_And_Bases_The_Block_Lacks_Should_Be_Repaired()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer(("Helped {M1} with {P1}", "project"), ("Fixed 42 bugs", "apps"), ("Same as before", "previous-note")))
            .EnqueueJson(Answer(("Worked on {P1}", "project")));

        var result = await Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = CallerBlockId });

        Model.Calls.Count.ShouldBe(2);
        var repair = Model.Calls[1].Messages.Last().Text!;
        repair.ShouldContain("suggestions[0].text: names a person");
        repair.ShouldContain("suggestions[1].text: contains a number that is not in the inputs");
        repair.ShouldContain("suggestions[2].basis: is not one of the allowed values");
        result.Suggestions.ShouldHaveSingleItem().Text.ShouldBe("Worked on Website");
    }

    [Fact]
    public async Task A_Provider_Failure_Should_Offer_The_Two_Templates()
    {
        await ReadyAsync();
        Model.EnqueueThrow(new AiProviderException(AiProviderErrorKind.ServerError, 503));

        var result = await Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = CallerBlockId });

        result.Meta.Status.ShouldBe(AiResultStatuses.Fallback);
        result.Suggestions.Select(s => s.Text).ShouldBe(new[] { "Worked on Website in Excel", "Worked on Website" });
        (await NoteOfAsync(CallerBlockId)).ShouldBe(SecretNote);
    }

    [Fact]
    public async Task The_Previous_Note_On_The_Same_Project_Should_Be_Offered_When_Notes_Are_Allowed()
    {
        await ReadyAsync(notes: true);
        var later = await InsertBlockAsync(CallerId, WebsiteId, Day.AddHours(10), 20, note: "Pricing table");

        var (_, bundle) = await BundleAsync(Feature<NoteSuggestionsFeature>(), new AiNoteSuggestionsInput { ActivityId = later });

        var notes = bundle.Lists[NoteSuggestionsFeature.NotesList]!.AsObject();
        bundle.Labels[AiFeatureOutput.Value(notes, "current")!].ShouldBe("Pricing table");
        bundle.Labels[AiFeatureOutput.Value(notes, "previous")!].ShouldBe(SecretNote);
        NoteSuggestionsFeature.AvailableBases(bundle).ShouldContain(AiNoteSuggestionBases.PreviousNote);
    }

    [Fact]
    public async Task Another_Members_Block_Should_Be_Suggested_For_An_Owner_Who_Sees_It()
    {
        await ReadyAsync();
        Model.EnqueueJson(Answer(("Worked on {P1}", "project")));

        var result = await Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = AyeshaBlockId });

        result.Suggestions.ShouldHaveSingleItem().Text.ShouldBe("Worked on Client portal");
        var row = (await RowsAsync()).ShouldHaveSingleItem();
        row.Audience.ShouldBe(AiUsageRecord.MemberAudience);
        row.SubjectUserId.ShouldBe(AyeshaId);
        ShouldHoldNothingPrivate(Model.Calls.ShouldHaveSingleItem());
    }
}

/// <summary>A worker without Team.Manage or ViewAll: their own recent blocks only.</summary>
public abstract class NoteSuggestionsWorkerTests<TStartupModule> : AiFeatureTestBase<TStartupModule>
    where TStartupModule : IAbpModule
{
    protected override string CallerRole => WorkspaceRoles.Member;

    protected override void AfterAddApplication(IServiceCollection services)
    {
        base.AfterAddApplication(services);
        services.DenyPermissions(TrackerPermissions.Activities.ViewAll, TrackerPermissions.Reporting.Default, TrackerPermissions.Team.Manage);
    }

    [Fact]
    public async Task An_Old_Block_Should_Be_Forbidden_And_Another_Members_Block_Not_Found()
    {
        await ReadyAsync();
        var old = await InsertBlockAsync(CallerId, WebsiteId, DateTime.UtcNow.Date.AddDays(-40).AddHours(9), 30);

        await Should.ThrowAsync<AbpAuthorizationException>(() => Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = old }));
        await Should.ThrowAsync<EntityNotFoundException>(() => Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = AyeshaBlockId }));
        await Should.ThrowAsync<EntityNotFoundException>(() => Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = Guid.NewGuid() }));
        Model.Calls.ShouldBeEmpty();
        (await RowsAsync()).ShouldBeEmpty();
    }

    [Fact]
    public async Task Their_Own_Recent_Block_Should_Be_Suggested()
    {
        await ReadyAsync();
        Model.EnqueueJson(Json(new { suggestions = new[] { new { text = "Worked on {P1}", basis = "project" } } }));

        var result = await Ai.CreateNoteSuggestionsAsync(new AiNoteSuggestionsInput { ActivityId = CallerBlockId });

        result.Suggestions.ShouldHaveSingleItem().Text.ShouldBe("Worked on Website");
    }
}
