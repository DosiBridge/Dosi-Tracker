using Shouldly;
using Xunit;

namespace Dosi.Tracker.Activities;

/* Pure unit tests for recognizing a capture's image format from its magic bytes. */
public class CaptureImageFormatTests
{
    [Fact]
    public void Should_Recognize_A_Jpeg()
    {
        CaptureImageFormat.DetectContentType(new byte[] { 0xFF, 0xD8, 0xFF }).ShouldBe("image/jpeg");
        CaptureImageFormat.DetectContentType(new byte[] { 0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10 }).ShouldBe("image/jpeg");
    }

    [Fact]
    public void Should_Recognize_A_Png()
    {
        CaptureImageFormat.DetectContentType(new byte[] { 0x89, 0x50, 0x4E, 0x47 }).ShouldBe("image/png");
        CaptureImageFormat.DetectContentType(new byte[] { 0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A }).ShouldBe("image/png");
    }

    [Theory]
    [InlineData(new byte[0])]
    [InlineData(new byte[] { 0xFF, 0xD8 })]
    [InlineData(new byte[] { 0x89, 0x50, 0x4E })]
    [InlineData(new byte[] { 0x47, 0x49, 0x46, 0x38, 0x39, 0x61 })] // GIF89a
    [InlineData(new byte[] { 0x3C, 0x73, 0x76, 0x67 })]             // "<svg"
    [InlineData(new byte[] { 0x52, 0x49, 0x46, 0x46 })]             // RIFF (WebP)
    public void Should_Not_Recognize_Anything_Else(byte[] bytes)
    {
        CaptureImageFormat.DetectContentType(bytes).ShouldBeNull();
    }

    [Theory]
    [InlineData("image/png", ".png")]
    [InlineData("IMAGE/PNG", ".png")]
    [InlineData("image/jpeg", ".jpg")]
    [InlineData(null, ".jpg")]
    public void FileExtension_Should_Follow_The_Content_Type(string? contentType, string expected)
    {
        CaptureImageFormat.FileExtension(contentType).ShouldBe(expected);
    }
}
