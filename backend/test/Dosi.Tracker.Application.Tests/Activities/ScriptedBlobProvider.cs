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

    /// <summary>Number of times DeleteAsync was invoked on this provider.</summary>
    public int DeleteCount => Volatile.Read(ref _deleteCount);

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

    private int _getCount;

    /// <summary>Number of times GetOrNullAsync was invoked on this provider.</summary>
    public int GetCount => Volatile.Read(ref _getCount);

    /// <summary>Chronological log of blob names requested via GetOrNullAsync.</summary>
    public ConcurrentQueue<string> GetAttempts { get; } = new();

    /// <summary>Called with the 1-based number of the get and the blob name, before the blob is retrieved.</summary>
    public Func<int, string, Task>? BeforeGet { get; set; }

    public override Task<bool> ExistsAsync(BlobProviderExistsArgs args) =>
        Task.FromResult(Blobs.ContainsKey(args.BlobName));

    public override async Task<Stream?> GetOrNullAsync(BlobProviderGetArgs args)
    {
        var getNumber = Interlocked.Increment(ref _getCount);
        GetAttempts.Enqueue(args.BlobName);
        if (BeforeGet != null)
        {
            await BeforeGet(getNumber, args.BlobName);
        }

        return Blobs.TryGetValue(args.BlobName, out var bytes) ? new MemoryStream(bytes) : null;
    }
}
