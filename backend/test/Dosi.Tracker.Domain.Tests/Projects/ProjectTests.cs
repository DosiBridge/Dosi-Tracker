using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Projects;

public class ProjectTests
{
    [Fact]
    public void New_Project_Should_Have_Sensible_Defaults()
    {
        var project = new Project(Guid.NewGuid(), null, "Website Redesign");

        project.Title.ShouldBe("Website Redesign");
        project.IntervalMinutes.ShouldBe(10);
        project.Color.ShouldBe("#006bff");
        project.IsArchived.ShouldBeFalse();
        project.Members.ShouldNotBeNull();
        project.Members.ShouldBeEmpty();
    }

    [Fact]
    public void New_Project_Should_Apply_PrivacyFirst_Tracking_Defaults()
    {
        // Privacy-first: everything on EXCEPT the webcam. Pinned exactly so a flipped default
        // (e.g. webcam silently on) is caught.
        var project = new Project(Guid.NewGuid(), null, "Website");

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
        var project = new Project(Guid.NewGuid(), null, "Website");

        project.Archive();
        project.IsArchived.ShouldBeTrue();

        project.Unarchive();
        project.IsArchived.ShouldBeFalse();
    }

    [Fact]
    public void New_ProjectMember_Should_Default_To_Worker_Role()
    {
        var member = new ProjectMember(Guid.NewGuid(), null, Guid.NewGuid(), Guid.NewGuid());

        member.Role.ShouldBe("Worker");
    }
}
