using System;
using System.Collections.Concurrent;
using System.Diagnostics;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Extensions.Options;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// Provider calls in flight on this API instance: at most <c>AI:Concurrency:Global</c> in all and
/// <c>AI:Concurrency:PerWorkspace</c> per workspace, so one busy workspace cannot take every slot. A request waits at
/// most <see cref="WaitTimeout"/> for its slots, then is refused (<c>Tracker:AiRateLimited</c> busy). A singleton; the
/// limits are read once, when it is created.
/// </summary>
public sealed class AiConcurrencyGate : IDisposable
{
    /// <summary>The longest a request waits for its slots.</summary>
    public static readonly TimeSpan DefaultWaitTimeout = TimeSpan.FromSeconds(2);

    /// <summary>The retry-after of a busy refusal.</summary>
    public const int BusyRetryAfterSeconds = 5;

    private readonly SemaphoreSlim _global;
    private readonly ConcurrentDictionary<Guid, SemaphoreSlim> _perWorkspace = new();

    public AiConcurrencyGate(IOptions<AiOptions> options)
        : this(options, DefaultWaitTimeout)
    {
    }

    internal AiConcurrencyGate(IOptions<AiOptions> options, TimeSpan waitTimeout)
    {
        var concurrency = options.Value.Concurrency ?? new AiConcurrencyOptions();
        GlobalLimit = Math.Max(1, concurrency.Global);
        PerWorkspaceLimit = Math.Max(1, Math.Min(concurrency.PerWorkspace, GlobalLimit));
        WaitTimeout = waitTimeout;
        _global = new SemaphoreSlim(GlobalLimit, GlobalLimit);
    }

    public int GlobalLimit { get; }

    public int PerWorkspaceLimit { get; }

    public TimeSpan WaitTimeout { get; }

    /// <summary>Takes a workspace slot and a global slot (waiting up to <see cref="WaitTimeout"/> for both); dispose the
    /// result to give them back.</summary>
    public async Task<IDisposable> AcquireAsync(Guid workspaceId, string featureId, CancellationToken cancellationToken = default)
    {
        var workspace = _perWorkspace.GetOrAdd(workspaceId, _ => new SemaphoreSlim(PerWorkspaceLimit, PerWorkspaceLimit));
        var waited = Stopwatch.StartNew();

        if (!await workspace.WaitAsync(WaitTimeout, cancellationToken))
        {
            throw Busy(featureId);
        }

        try
        {
            var remaining = WaitTimeout - waited.Elapsed;
            if (!await _global.WaitAsync(remaining > TimeSpan.Zero ? remaining : TimeSpan.Zero, cancellationToken))
            {
                throw Busy(featureId);
            }
        }
        catch
        {
            workspace.Release();
            throw;
        }

        return new Slot(workspace, _global);
    }

    public void Dispose()
    {
        _global.Dispose();
        foreach (var semaphore in _perWorkspace.Values)
        {
            semaphore.Dispose();
        }
    }

    private static Exception Busy(string featureId) =>
        AiErrors.RateLimited(AiErrors.BusyReason, BusyRetryAfterSeconds, featureId);

    private sealed class Slot : IDisposable
    {
        private SemaphoreSlim? _workspace;
        private SemaphoreSlim? _global;

        public Slot(SemaphoreSlim workspace, SemaphoreSlim global)
        {
            _workspace = workspace;
            _global = global;
        }

        public void Dispose()
        {
            Interlocked.Exchange(ref _global, null)?.Release();
            Interlocked.Exchange(ref _workspace, null)?.Release();
        }
    }
}
