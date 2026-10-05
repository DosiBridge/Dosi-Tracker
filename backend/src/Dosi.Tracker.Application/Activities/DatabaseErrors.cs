using System;
using System.Data.Common;

namespace Dosi.Tracker.Activities;

/// <summary>Classifies database errors without depending on a database provider.</summary>
internal static class DatabaseErrors
{
    /// <summary>SQLSTATE of a unique-constraint violation (SQL standard; PostgreSQL).</summary>
    public const string UniqueViolationSqlState = "23505";

    /// <summary>
    /// Whether <paramref name="exception"/> may be a unique-index violation: a database error with SQLSTATE 23505,
    /// or one from a provider that reports no SQLSTATE (SQLite). A database error with any other SQLSTATE — a lost
    /// connection, a timeout, a serialization failure — is not, and neither is anything that is not a database
    /// error (a failed blob upload, for one). Callers confirm a suspected duplicate by finding the row that won.
    /// </summary>
    public static bool MayBeUniqueViolation(Exception? exception)
    {
        for (var current = exception; current != null; current = current.InnerException)
        {
            if (current is DbException dbException)
            {
                return string.IsNullOrEmpty(dbException.SqlState) ||
                       string.Equals(dbException.SqlState, UniqueViolationSqlState, StringComparison.Ordinal);
            }
        }

        return false;
    }
}
