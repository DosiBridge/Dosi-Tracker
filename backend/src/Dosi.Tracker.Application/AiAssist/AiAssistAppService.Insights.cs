using System;
using System.Threading.Tasks;
using Dosi.Tracker.AiAssist.Features;
using Dosi.Tracker.AiAssist.Pipeline;
using Microsoft.Extensions.DependencyInjection;
using Volo.Abp.Uow;

namespace Dosi.Tracker.AiAssist;

/// <summary>
/// The insight cards. Each delegates to <see cref="AiInsightRunner"/> with a fresh feature instance (features keep what
/// one request authorized, such as its project, between the runner's steps). None holds a database transaction while the
/// provider is called.
/// </summary>
public partial class AiAssistAppService
{
    protected AiInsightRunner Runner => LazyServiceProvider.LazyGetRequiredService<AiInsightRunner>();

    [UnitOfWork(isTransactional: false)]
    public virtual Task<AiPeriodSummaryDto> CreatePeriodSummaryAsync(AiPeriodSummaryInput input) =>
        Runner.RunAsync(NewFeature<PeriodSummaryFeature>(), input);

    [UnitOfWork(isTransactional: false)]
    public virtual Task<AiMemberDaySummaryDto> CreateMemberDaySummaryAsync(AiMemberDaySummaryInput input) =>
        Runner.RunAsync(NewFeature<MemberDaySummaryFeature>(), input);

    [UnitOfWork(isTransactional: false)]
    public virtual Task<AiReportExplanationDto> CreateReportExplanationAsync(AiReportExplanationInput input) =>
        Runner.RunAsync(NewFeature<ReportExplanationFeature>(), input);

    /// <summary>A new instance of a (transient) feature from the request's container: never a cached one.</summary>
    protected virtual TFeature NewFeature<TFeature>()
        where TFeature : notnull =>
        LazyServiceProvider.LazyGetRequiredService<IServiceProvider>().GetRequiredService<TFeature>();
}
