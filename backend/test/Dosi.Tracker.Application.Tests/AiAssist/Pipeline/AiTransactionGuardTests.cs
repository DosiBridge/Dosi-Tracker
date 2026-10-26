using System;
using NSubstitute;
using Shouldly;
using Volo.Abp.Uow;
using Xunit;

namespace Dosi.Tracker.AiAssist.Pipeline;

/// <summary>
/// The runner never calls a provider while a database transaction is open (the EF test module turns transactions off
/// everywhere, so the rule itself is tested here, on the unit of work it inspects).
/// </summary>
public class AiTransactionGuardTests
{
    private static IUnitOfWork UnitOfWork(bool transactional)
    {
        var uow = Substitute.For<IUnitOfWork>();
        uow.Options.Returns(new AbpUnitOfWorkOptions { IsTransactional = transactional });
        return uow;
    }

    [Fact]
    public void A_Transactional_Unit_Of_Work_Should_Be_Refused()
    {
        var refused = Should.Throw<InvalidOperationException>(() => AiInsightRunner.EnsureNoTransaction(UnitOfWork(transactional: true)));

        refused.Message.ShouldContain("[UnitOfWork(isTransactional: false)]");
    }

    [Fact]
    public void No_Unit_Of_Work_Or_A_Non_Transactional_One_Should_Pass()
    {
        Should.NotThrow(() => AiInsightRunner.EnsureNoTransaction(null));
        Should.NotThrow(() => AiInsightRunner.EnsureNoTransaction(UnitOfWork(transactional: false)));
    }
}
