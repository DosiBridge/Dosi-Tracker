using System;
using System.Threading.Tasks;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Guids;

namespace Dosi.Tracker.SaaS;

/// <summary>Seeds the platform's default plans (host-level; plans are shared across tenants).</summary>
public class PlanDataSeedContributor : IDataSeedContributor, ITransientDependency
{
    private readonly IRepository<Plan, Guid> _planRepository;
    private readonly IGuidGenerator _guidGenerator;

    public PlanDataSeedContributor(
        IRepository<Plan, Guid> planRepository,
        IGuidGenerator guidGenerator)
    {
        _planRepository = planRepository;
        _guidGenerator = guidGenerator;
    }

    public async Task SeedAsync(DataSeedContext context)
    {
        // Plans live on the host side only.
        if (context.TenantId != null)
        {
            return;
        }

        if (await _planRepository.AnyAsync())
        {
            return;
        }

        await _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Free", 0m, maxSeats: 3, trialDays: 0));
        await _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Starter", 6m, maxSeats: 25, trialDays: 14));
        await _planRepository.InsertAsync(new Plan(_guidGenerator.Create(), "Business", 12m, maxSeats: 100, trialDays: 14));
    }
}
