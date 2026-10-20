using System;
using System.Collections.Concurrent;
using System.IO;
using System.Threading;
using System.Threading.Tasks;
using Volo.Abp.BlobStoring;
using Volo.Abp.DependencyInjection;

namespace Dosi.Tracker.Activities;

/// <summary>
/// An in-memory blob provider for tests that need storage to misbehave: <see cref="BeforeSave"/> runs before each
/// save and may throw (a failing object store) or act as a concurrent request.
/// </summary>
[DisableConventionalRegistration]
public sealed class ScriptedBlobProvider : BlobProviderBase
{
    private int _saveCount;

    public ConcurrentDictionary<string, byte[]> Blobs { get; } = new();

    /// <summary>Called with the 1-based number of the save and the blob name, before the blob is stored.</summary>
    public Func<int, string, Task>? BeforeSave { get; set; }

    private int _deleteCount;

    /// <summary>Called with the 1-based number of the delete and the blob name, before the blob is removed.</summary>
    public Func<int, string, Task>? BeforeDelete { get; set; }

    public override async Task SaveAsync(BlobProviderSaveArgs args)
    {
        var saveNumber = Interlocked.Increment(ref _saveCount);
        if (BeforeSave != null)
        {
            await BeforeSave(saveNumber, args.BlobName);
        }

        if (!args.OverrideExisting && Blobs.ContainsKey(args.BlobName))
        {
            throw new BlobAlreadyExistsException($"Blob '{args.BlobName}' already exists.");
        }

        Blobs[args.BlobName] = await args.BlobStream.GetAllBytesAsync(args.CancellationToken);
    }

    public override async Task<bool> DeleteAsync(BlobProviderDeleteArgs args)
    {
        var deleteNumber = Interlocked.Increment(ref _deleteCount);
        if (BeforeDelete != null)
        {
            await BeforeDelete(deleteNumber, args.BlobName);
        }

        return Blobs.TryRemove(args.BlobName, out _);
    }

    public override Task<bool> ExistsAsync(BlobProviderExistsArgs args) =>
        Task.FromResult(Blobs.ContainsKey(args.BlobName));

    public override Task<Stream?> GetOrNullAsync(BlobProviderGetArgs args) =>
        Task.FromResult<Stream?>(Blobs.TryGetValue(args.BlobName, out var bytes) ? new MemoryStream(bytes) : null);
}
