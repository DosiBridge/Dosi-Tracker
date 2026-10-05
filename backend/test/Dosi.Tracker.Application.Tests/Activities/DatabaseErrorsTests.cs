using System;
using System.Data.Common;
using System.IO;
using Shouldly;
using Xunit;

namespace Dosi.Tracker.Activities;

/* Pure unit tests for telling a unique-index violation (the duplicate-upload race) from every other failure. */
public class DatabaseErrorsTests
{
    private sealed class TestDbException : DbException
    {
        private readonly string? _sqlState;

        public TestDbException(string? sqlState)
            : base("A database error.")
        {
            _sqlState = sqlState;
        }

        public override string? SqlState => _sqlState;
    }

    [Fact]
    public void Should_Recognize_A_Unique_Violation_By_Its_SqlState()
    {
        DatabaseErrors.MayBeUniqueViolation(new TestDbException("23505")).ShouldBeTrue();
    }

    [Fact]
    public void Should_Find_The_Database_Error_Inside_Wrapping_Exceptions()
    {
        // EF Core reports a failed save as a DbUpdateException wrapping the provider's exception.
        var wrapped = new InvalidOperationException("An error occurred while saving the entity changes.",
            new TestDbException("23505"));

        DatabaseErrors.MayBeUniqueViolation(wrapped).ShouldBeTrue();
    }

    [Theory]
    [InlineData("08006")] // connection failure
    [InlineData("57014")] // query canceled (timeout)
    [InlineData("40001")] // serialization failure
    [InlineData("23503")] // foreign-key violation
    public void Should_Not_Mistake_Another_Database_Error_For_A_Duplicate(string sqlState)
    {
        DatabaseErrors.MayBeUniqueViolation(new InvalidOperationException("Save failed.", new TestDbException(sqlState)))
            .ShouldBeFalse();
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    public void Should_Leave_A_Database_Error_Without_SqlState_To_Be_Confirmed(string? sqlState)
    {
        // SQLite reports no SQLSTATE: the caller confirms a suspected duplicate by finding the row that won.
        DatabaseErrors.MayBeUniqueViolation(new TestDbException(sqlState)).ShouldBeTrue();
    }

    [Fact]
    public void Should_Not_Treat_A_Storage_Failure_As_A_Duplicate()
    {
        DatabaseErrors.MayBeUniqueViolation(new IOException("The object store is unreachable.")).ShouldBeFalse();
        DatabaseErrors.MayBeUniqueViolation(new InvalidOperationException("Wrapped", new TimeoutException())).ShouldBeFalse();
        DatabaseErrors.MayBeUniqueViolation(null).ShouldBeFalse();
    }
}
