using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.Extensions.Logging;

namespace Dosi.Tracker.AiAssist.Fakes;

/// <summary>
/// Keeps every formatted log line (and the exception's text, when one is logged) of the application under test, so a
/// test can assert what never reaches the logs. Register it with <c>services.AddSingleton&lt;ILoggerProvider&gt;(provider)</c>.
/// </summary>
public sealed class CapturingLoggerProvider : ILoggerProvider
{
    private readonly List<CapturedLog> _entries = new();

    public IReadOnlyList<CapturedLog> Entries
    {
        get
        {
            lock (_entries)
            {
                return _entries.ToList();
            }
        }
    }

    /// <summary>Every captured line and exception text, joined.</summary>
    public string AllText => string.Join("\n", Entries.Select(e => e.Message + (e.Exception == null ? string.Empty : "\n" + e.Exception)));

    public ILogger CreateLogger(string categoryName) => new Logger(this, categoryName);

    public void Dispose()
    {
    }

    private void Add(CapturedLog entry)
    {
        lock (_entries)
        {
            _entries.Add(entry);
        }
    }

    private sealed class Logger : ILogger
    {
        private readonly CapturingLoggerProvider _provider;
        private readonly string _category;

        public Logger(CapturingLoggerProvider provider, string category)
        {
            _provider = provider;
            _category = category;
        }

        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => null;

        public bool IsEnabled(LogLevel logLevel) => true;

        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            _provider.Add(new CapturedLog(_category, logLevel, formatter(state, exception), exception?.ToString()));
        }
    }
}

public sealed record CapturedLog(string Category, LogLevel Level, string Message, string? Exception);
