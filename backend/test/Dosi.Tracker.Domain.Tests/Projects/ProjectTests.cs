using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Projects;

public class ProjectTests
{
    [Fact]
    public void New_Project_Should_Have_Sensible_Defaults()
    {
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Website Redesign");

        project.Title.ShouldBe("Website Redesign");
        project.IntervalMinutes.ShouldBe(10);
        project.Color.ShouldBe("#006bff");
        project.IsArchived.ShouldBeFalse();
        project.ArchivedAt.ShouldBeNull();
        project.Members.ShouldNotBeNull();
        project.Members.ShouldBeEmpty();
    }

    [Fact]
    public void New_Project_Should_Detect_Idle_After_5_Minutes_And_Not_Blur_Screenshots()
    {
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Website");

        project.IdleThresholdMinutes.ShouldBe(5);
        project.BlurScreenshots.ShouldBeFalse();
    }

    [Fact]
    public void New_Project_Should_Apply_PrivacyFirst_Tracking_Defaults()
    {
        // Privacy-first: everything on EXCEPT the webcam. Pinned exactly so a flipped default
        // (e.g. webcam silently on) is caught.
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Website");

        project.AllowScreenshot.ShouldBeTrue();
        project.AllowWebcam.ShouldBeFalse();
        project.AllowKeyboard.ShouldBeTrue();
        project.AllowMouse.ShouldBeTrue();
        project.AllowActiveWindow.ShouldBeTrue();
        project.AllowRunningPrograms.ShouldBeTrue();
    }

    [Fact]
    public void Archive_And_Unarchive_Should_Toggle_The_Flag()
    {
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Website");
        var archivedAt = new DateTime(2026, 7, 20, 17, 5, 0, DateTimeKind.Utc);

        project.Archive(archivedAt);
        project.IsArchived.ShouldBeTrue();
        project.ArchivedAt.ShouldBe(archivedAt);

        project.Unarchive();
        project.IsArchived.ShouldBeFalse();
        project.ArchivedAt.ShouldBeNull();
    }

    [Fact]
    public void Archiving_Again_Should_Keep_The_Original_Archive_Moment()
    {
        // Blocks are accepted up to ArchivedAt, so a repeated archive must never move the cut-off later.
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Website");
        var first = new DateTime(2026, 7, 20, 17, 5, 0, DateTimeKind.Utc);

        project.Archive(first);
        project.Archive(first.AddHours(3));

        project.IsArchived.ShouldBeTrue();
        project.ArchivedAt.ShouldBe(first);
    }

    [Fact]
    public void Archive_Should_Stamp_A_Legacy_Archived_Project_That_Has_No_Moment()
    {
        // Archived before ArchivedAt existed (flag only): archiving again records the moment.
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Website") { IsArchived = true };
        var archivedAt = new DateTime(2026, 7, 20, 17, 5, 0, DateTimeKind.Utc);

        project.Archive(archivedAt);

        project.ArchivedAt.ShouldBe(archivedAt);
    }

    [Fact]
    public void Unarchive_Then_Archive_Should_Record_The_New_Moment()
    {
        var project = new Project(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, "Website");
        var first = new DateTime(2026, 7, 20, 17, 5, 0, DateTimeKind.Utc);

        project.Archive(first);
        project.Unarchive();
        project.Archive(first.AddDays(2));

        project.ArchivedAt.ShouldBe(first.AddDays(2));
    }

    [Fact]
    public void New_ProjectMemberRate_Should_Keep_Its_Effective_Date_And_Rate()
    {
        var tenantId = Guid.NewGuid();
        var projectId = Guid.NewGuid();
        var userId = Guid.NewGuid();
        var effectiveFrom = new DateTime(2026, 10, 1, 0, 0, 0, DateTimeKind.Utc);

        var rate = new ProjectMemberRate(Guid.NewGuid(), tenantId, projectId, userId, 30.5m, effectiveFrom);

        rate.WorkspaceId.ShouldBe(tenantId);
        rate.ProjectId.ShouldBe(projectId);
        rate.UserId.ShouldBe(userId);
        rate.HourlyRate.ShouldBe(30.5m);
        rate.EffectiveFrom.ShouldBe(effectiveFrom);
    }

    [Fact]
    public void New_ProjectMember_Should_Default_To_Worker_Role()
    {
        var member = new ProjectMember(Guid.NewGuid(), TrackerTestConsts.WorkspaceId, Guid.NewGuid(), Guid.NewGuid());

        member.Role.ShouldBe("Worker");
    }
}
