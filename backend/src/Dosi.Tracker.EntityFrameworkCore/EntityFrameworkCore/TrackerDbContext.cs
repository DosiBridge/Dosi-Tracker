using Microsoft.EntityFrameworkCore;
using Volo.Abp.AuditLogging.EntityFrameworkCore;
using Volo.Abp.BackgroundJobs.EntityFrameworkCore;
using Volo.Abp.BlobStoring.Database.EntityFrameworkCore;
using Volo.Abp.Data;
using Volo.Abp.DependencyInjection;
using Volo.Abp.EntityFrameworkCore;
using Volo.Abp.EntityFrameworkCore.Modeling;
using Volo.Abp.FeatureManagement.EntityFrameworkCore;
using Volo.Abp.Identity;
using Volo.Abp.Identity.EntityFrameworkCore;
using Volo.Abp.PermissionManagement.EntityFrameworkCore;
using Volo.Abp.SettingManagement.EntityFrameworkCore;
using Volo.Abp.OpenIddict.EntityFrameworkCore;
using System;
using System.Linq.Expressions;
using Dosi.Tracker.Workspaces;
using Microsoft.EntityFrameworkCore.ChangeTracking;
using Microsoft.EntityFrameworkCore.Metadata;
using Microsoft.EntityFrameworkCore.Metadata.Builders;
using Volo.Abp;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Devices;
using Dosi.Tracker.Presence;

namespace Dosi.Tracker.EntityFrameworkCore;

[ReplaceDbContext(typeof(IIdentityDbContext))]
[ConnectionStringName("Default")]
public class TrackerDbContext :
    AbpDbContext<TrackerDbContext>,
    IIdentityDbContext
{
    /* Add DbSet properties for your Aggregate Roots / Entities here. */
    public DbSet<Project> Projects { get; set; }
    public DbSet<ProjectMember> ProjectMembers { get; set; }
    public DbSet<ProjectMemberRate> ProjectMemberRates { get; set; }
    public DbSet<ProjectTask> ProjectTasks { get; set; }
    public DbSet<Activity> Activities { get; set; }
    public DbSet<Screenshot> Screenshots { get; set; }
    public DbSet<AgentPresence> AgentPresences { get; set; }
    public DbSet<DeviceRevocation> DeviceRevocations { get; set; }

    // New Domains
    public DbSet<Dosi.Tracker.SaaS.Plan> Plans { get; set; }
    public DbSet<Dosi.Tracker.SaaS.Subscription> Subscriptions { get; set; }
    public DbSet<Dosi.Tracker.Billing.Invoice> Invoices { get; set; }
    public DbSet<Dosi.Tracker.Notifications.Notification> Notifications { get; set; }
    public DbSet<Workspace> Workspaces { get; set; }
    public DbSet<WorkspaceMember> WorkspaceMembers { get; set; }
    public DbSet<WorkspaceInvitation> WorkspaceInvitations { get; set; }

    #region Entities from the modules

    /* Notice: We only implemented IIdentityProDbContext and ISaasDbContext
     * and replaced them for this DbContext. This allows you to perform JOIN
     * queries for the entities of these modules over the repositories easily. You
     * typically don't need that for other modules. But, if you need, you can
     * implement the DbContext interface of the needed module and use ReplaceDbContext
     * attribute just like IIdentityProDbContext and ISaasDbContext.
     *
     * More info: Replacing a DbContext of a module ensures that the related module
     * uses this DbContext on runtime. Otherwise, it will use its own DbContext class.
     */

    // Identity
    public DbSet<IdentityUser> Users { get; set; }
    public DbSet<IdentityRole> Roles { get; set; }
    public DbSet<IdentityClaimType> ClaimTypes { get; set; }
    public DbSet<OrganizationUnit> OrganizationUnits { get; set; }
    public DbSet<IdentitySecurityLog> SecurityLogs { get; set; }
    public DbSet<IdentityLinkUser> LinkUsers { get; set; }
    public DbSet<IdentityUserDelegation> UserDelegations { get; set; }
    public DbSet<IdentitySession> Sessions { get; set; }

    #endregion

    public TrackerDbContext(DbContextOptions<TrackerDbContext> options)
        : base(options)
    {

    }

    protected override void OnModelCreating(ModelBuilder builder)
    {
        base.OnModelCreating(builder);

        /* Include modules to your migration db context */

        builder.ConfigurePermissionManagement();
        builder.ConfigureSettingManagement();
        builder.ConfigureBackgroundJobs();
        builder.ConfigureAuditLogging();
        builder.ConfigureFeatureManagement();
        builder.ConfigureIdentity();
        builder.ConfigureOpenIddict();
        builder.ConfigureBlobStoring();

        builder.Entity<Workspace>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Workspaces", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Name).IsRequired().HasMaxLength(WorkspaceConsts.MaxNameLength);
            b.Property(x => x.NormalizedName).IsRequired().HasMaxLength(WorkspaceConsts.MaxNameLength);
            b.Property(x => x.IsArchived).HasDefaultValue(false);
            // The backstop for concurrent creations of the same name (Tracker:DuplicateWorkspaceName).
            b.HasIndex(x => x.NormalizedName).IsUnique();
            b.HasIndex(x => x.IsArchived);
        });

        builder.Entity<WorkspaceMember>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "WorkspaceMembers", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Role).IsRequired().HasMaxLength(16);
            b.Property(x => x.Status).IsRequired().HasMaxLength(16);
            b.HasIndex(x => new { x.WorkspaceId, x.UserId }).IsUnique();
            b.HasIndex(x => x.UserId);
            b.HasOne<Workspace>().WithMany().HasForeignKey(x => x.WorkspaceId).IsRequired();
        });

        builder.Entity<WorkspaceInvitation>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "WorkspaceInvitations", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Email).IsRequired().HasMaxLength(256);
            b.Property(x => x.NormalizedEmail).IsRequired().HasMaxLength(256);
            b.Property(x => x.Role).IsRequired().HasMaxLength(16);
            b.Property(x => x.ProjectRole).HasMaxLength(64);
            b.Property(x => x.TokenHash).IsRequired().HasMaxLength(64);
            b.HasIndex(x => x.TokenHash).IsUnique();
            b.HasIndex(x => new { x.WorkspaceId, x.NormalizedEmail });
            b.HasOne<Workspace>().WithMany().HasForeignKey(x => x.WorkspaceId).IsRequired();
        });

        /* Configure your own tables/entities inside here */

        builder.Entity<Project>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Projects", TrackerConsts.DbSchema);
            b.ConfigureByConvention(); //auto configure for the base class props
            b.Property(x => x.Title).IsRequired().HasMaxLength(ProjectConsts.MaxTitleLength);
            b.Property(x => x.Color).IsRequired().HasMaxLength(ProjectConsts.MaxColorLength);
            b.Property(x => x.Description).HasMaxLength(ProjectConsts.MaxDescriptionLength);
            b.HasMany(x => x.Members).WithOne().HasForeignKey(x => x.ProjectId).IsRequired();
        });

        builder.Entity<ProjectMember>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "ProjectMembers", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Role).IsRequired().HasMaxLength(64);
            // A person joins a project once: the backstop when two adds of the same person race past the service's
            // check (Tracker:DuplicateProjectMember).
            b.HasIndex(x => new { x.WorkspaceId, x.ProjectId, x.UserId }).IsUnique();
        });

        builder.Entity<ProjectMemberRate>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "ProjectMemberRates", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            // Payroll looks up the rate in effect at a block's start: the member's latest EffectiveFrom <= StartedAt.
            b.HasIndex(x => new { x.WorkspaceId, x.ProjectId, x.UserId, x.EffectiveFrom });
        });

        builder.Entity<ProjectTask>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "ProjectTasks", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Title).IsRequired().HasMaxLength(ProjectTaskConsts.MaxTitleLength);
            b.Property(x => x.Description).HasMaxLength(ProjectTaskConsts.MaxDescriptionLength);
            b.HasIndex(x => new { x.WorkspaceId, x.ProjectId });
            b.HasIndex(x => new { x.WorkspaceId, x.ProjectId, x.IsCompleted });
        });

        builder.Entity<Activity>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Activities", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Platform).HasMaxLength(ActivityConsts.MaxPlatformLength);
            b.Property(x => x.OsVersion).HasMaxLength(ActivityConsts.MaxOsVersionLength);
            b.Property(x => x.AgentVersion).HasMaxLength(ActivityConsts.MaxAgentVersionLength);
            b.Property(x => x.DeviceName).HasMaxLength(ActivityConsts.MaxDeviceNameLength);
            b.Property(x => x.TimeZone).HasMaxLength(ActivityConsts.MaxTimeZoneLength);
            b.Property(x => x.EndReason).HasMaxLength(ActivityConsts.MaxEndReasonLength);
            b.HasIndex(x => new { x.WorkspaceId, x.UserId, x.StartedAt }).IsDescending(false, false, true);
            b.HasIndex(x => new { x.WorkspaceId, x.ProjectId, x.StartedAt }).IsDescending(false, false, true);
            b.HasIndex(x => new { x.WorkspaceId, x.StartedAt }).IsDescending(false, true); // Workspace-wide reports and lists
            // Idempotency, per member: the backstop for concurrent retries across server instances.
            b.HasIndex(x => new { x.WorkspaceId, x.UserId, x.ClientActivityId }).IsUnique();
            // Lookup only (not unique): a client id already used by another member is refused (ActivityClientIdConflict).
            b.HasIndex(x => new { x.WorkspaceId, x.ClientActivityId });
            b.HasMany(x => x.Screenshots).WithOne().HasForeignKey(x => x.ActivityId).IsRequired();
        });

        builder.Entity<Screenshot>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Screenshots", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.HasIndex(x => new { x.WorkspaceId, x.CapturedAt }).IsDescending(false, true);
            b.Property(x => x.StorageUrl).IsRequired().HasMaxLength(512);
            b.Property(x => x.Kind).IsRequired().HasMaxLength(16);
        });

        builder.Entity<AgentPresence>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "AgentPresences", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.DeviceName).HasMaxLength(ActivityConsts.MaxDeviceNameLength);
            b.Property(x => x.Platform).HasMaxLength(ActivityConsts.MaxPlatformLength);
            b.Property(x => x.OsVersion).HasMaxLength(ActivityConsts.MaxOsVersionLength);
            b.Property(x => x.AgentVersion).HasMaxLength(ActivityConsts.MaxAgentVersionLength);
            b.Property(x => x.Status).IsRequired().HasMaxLength(AgentPresenceConsts.MaxStatusLength);
            b.Property(x => x.CurrentApp).HasMaxLength(AgentPresenceConsts.MaxCurrentAppLength);
            b.Property(x => x.TimeZone).HasMaxLength(ActivityConsts.MaxTimeZoneLength);
            // One row per agent install.
            b.HasIndex(x => new { x.WorkspaceId, x.UserId, x.DeviceId }).IsUnique();
        });

        builder.Entity<DeviceRevocation>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "DeviceRevocations", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Reason).HasMaxLength(DeviceConsts.MaxReasonLength);
            b.HasIndex(x => new { x.WorkspaceId, x.UserId, x.DeviceId }).IsUnique();
        });

        // New Domains
        builder.Entity<Dosi.Tracker.SaaS.Plan>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Plans", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Name).IsRequired().HasMaxLength(64);
        });

        builder.Entity<Dosi.Tracker.SaaS.Subscription>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Subscriptions", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Status).IsRequired().HasMaxLength(32);
            b.HasIndex(x => x.WorkspaceId);
        });

        builder.Entity<Dosi.Tracker.Billing.Invoice>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Invoices", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Status).IsRequired().HasMaxLength(32);

            // At most one live invoice per workspace and billing period: the database backstop for concurrent sweeps.
            var periodIndex = b.HasIndex(x => new { x.WorkspaceId, x.BillingPeriodStart }).IsUnique();
            if (builder.IsUsingPostgreSql() || builder.IsUsingSqlite())
            {
                // Partial index (both providers support the filter): legacy invoices without a period and
                // soft-deleted invoices never block a new one.
                periodIndex.HasFilter("\"BillingPeriodStart\" IS NOT NULL AND \"IsDeleted\" = false");
            }
        });

        builder.Entity<Dosi.Tracker.Notifications.Notification>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Notifications", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Message).IsRequired().HasMaxLength(512);
            b.HasIndex(x => new { x.WorkspaceId, x.UserId });
        });
    }

    protected bool IsWorkspaceFilterEnabled => DataFilter?.IsEnabled<IWorkspaceScoped>() ?? false;

    protected Guid? CurrentWorkspaceId => LazyServiceProvider?.LazyGetService<ICurrentWorkspace>()?.Id;

    protected override bool ShouldFilterEntity<TEntity>(IMutableEntityType entityType)
    {
        return typeof(IWorkspaceScoped).IsAssignableFrom(typeof(TEntity)) || base.ShouldFilterEntity<TEntity>(entityType);
    }

    protected override Expression<Func<TEntity, bool>>? CreateFilterExpression<TEntity>(
        ModelBuilder modelBuilder,
        EntityTypeBuilder<TEntity> entityTypeBuilder)
    {
        var expression = base.CreateFilterExpression<TEntity>(modelBuilder, entityTypeBuilder);
        if (typeof(IWorkspaceScoped).IsAssignableFrom(typeof(TEntity)))
        {
            Expression<Func<TEntity, bool>> workspaceFilter =
                e => !IsWorkspaceFilterEnabled || EF.Property<Guid>(e, nameof(IWorkspaceScoped.WorkspaceId)) == CurrentWorkspaceId;
            expression = expression == null
                ? workspaceFilter
                : QueryFilterExpressionHelper.CombineExpressions(expression, workspaceFilter);
        }

        return expression;
    }

    /// <summary>A new workspace-scoped row without a workspace gets the current one; without a current workspace it
    /// is refused (<c>Tracker:WorkspaceRequired</c>).</summary>
    protected override void ApplyAbpConceptsForAddedEntity(EntityEntry entry)
    {
        if (entry.Entity is IWorkspaceScoped scoped && scoped.WorkspaceId == Guid.Empty)
        {
            var workspaceId = CurrentWorkspaceId ?? throw new BusinessException(TrackerDomainErrorCodes.WorkspaceRequired);
            entry.Property(nameof(IWorkspaceScoped.WorkspaceId)).CurrentValue = workspaceId;
        }

        base.ApplyAbpConceptsForAddedEntity(entry);
    }
}
