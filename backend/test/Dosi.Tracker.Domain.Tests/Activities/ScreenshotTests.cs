using System;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Activities;

public class ScreenshotTests
{
    [Fact]
    public void Constructor_Should_Set_Fields_And_Default_To_A_Jpeg_Screen_Capture()
    {
        var id = Guid.NewGuid();
        var activityId = Guid.NewGuid();
        var capturedAt = new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);

        var shot = new Screenshot(id, null, activityId, "blob-key", capturedAt, 2048);

        shot.Id.ShouldBe(id);
        shot.ActivityId.ShouldBe(activityId);
        shot.StorageUrl.ShouldBe("blob-key");
        shot.CapturedAt.ShouldBe(capturedAt);
        shot.SizeBytes.ShouldBe(2048);
        shot.ContentType.ShouldBe("image/jpeg"); // default
        shot.Kind.ShouldBe(Screenshot.ScreenKind); // default
    }

    [Fact]
    public void Constructor_Should_Accept_An_Explicit_Webcam_Capture()
    {
        var capturedAt = new DateTime(2026, 7, 20, 10, 0, 0, DateTimeKind.Utc);

        var shot = new Screenshot(
            Guid.NewGuid(), null, Guid.NewGuid(), "cam-blob", capturedAt, 512, "image/png", Screenshot.WebcamKind);

        shot.ContentType.ShouldBe("image/png");
        shot.Kind.ShouldBe(Screenshot.WebcamKind);
    }
}
