using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Linq;
using Microsoft.Extensions.Logging;

namespace Dosi.Tracker.Emailing;

/// <summary>A logger that keeps what was logged, for asserting what an email log line does — and does not — contain.</summary>
public sealed class ListLogger<T> : ILogger<T>
{
    private readonly ConcurrentQueue<(LogLevel Level, string Message)> _entries = new();

    public IReadOnlyList<(LogLevel Level, string Message)> Entries => _entries.ToList();

    public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;

    public bool IsEnabled(LogLevel logLevel) => true;

    public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
    {
        _entries.Enqueue((logLevel, formatter(state, exception)));
    }
}
