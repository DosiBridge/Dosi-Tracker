using System;

namespace Dosi.Tracker.Activities;

/// <summary>
/// The image formats a capture may have, recognized by their magic bytes rather than by the upload field they
/// came in (<c>screenshotPngBase64</c> carries JPEG or PNG, shared contract §1). Anything else is not stored.
/// </summary>
internal static class CaptureImageFormat
{
    public const string Jpeg = "image/jpeg";
    public const string Png = "image/png";

    /// <summary>The content type of <paramref name="bytes"/>: FF D8 FF is JPEG, 89 50 4E 47 is PNG; null otherwise.</summary>
    public static string? DetectContentType(ReadOnlySpan<byte> bytes)
    {
        if (bytes.Length >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF)
        {
            return Jpeg;
        }

        if (bytes.Length >= 4 && bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47)
        {
            return Png;
        }

        return null;
    }

    /// <summary>The file extension of a capture stored as <paramref name="contentType"/>.</summary>
    public static string FileExtension(string? contentType) =>
        string.Equals(contentType, Png, StringComparison.OrdinalIgnoreCase) ? ".png" : ".jpg";
}
