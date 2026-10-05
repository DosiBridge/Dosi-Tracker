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
using Volo.Abp.TenantManagement;
using Volo.Abp.TenantManagement.EntityFrameworkCore;
using Dosi.Tracker.Projects;
using Dosi.Tracker.Activities;
using Dosi.Tracker.Presence;

namespace Dosi.Tracker.EntityFrameworkCore;

[ReplaceDbContext(typeof(IIdentityDbContext))]
[ReplaceDbContext(typeof(ITenantManagementDbContext))]
[ConnectionStringName("Default")]
public class TrackerDbContext :
    AbpDbContext<TrackerDbContext>,
    ITenantManagementDbContext,
    IIdentityDbContext
{
    /* Add DbSet properties for your Aggregate Roots / Entities here. */
    public DbSet<Project> Projects { get; set; }
    public DbSet<ProjectMember> ProjectMembers { get; set; }
    public DbSet<ProjectMemberRate> ProjectMemberRates { get; set; }
    public DbSet<Activity> Activities { get; set; }
    public DbSet<Screenshot> Screenshots { get; set; }
    public DbSet<AgentPresence> AgentPresences { get; set; }

    // New Domains
    public DbSet<Dosi.Tracker.SaaS.Plan> Plans { get; set; }
    public DbSet<Dosi.Tracker.SaaS.Subscription> Subscriptions { get; set; }
    public DbSet<Dosi.Tracker.Billing.Invoice> Invoices { get; set; }
    public DbSet<Dosi.Tracker.Notifications.Notification> Notifications { get; set; }

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

    // Tenant Management
    public DbSet<Tenant> Tenants { get; set; }
    public DbSet<TenantConnectionString> TenantConnectionStrings { get; set; }

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
        builder.ConfigureTenantManagement();
        builder.ConfigureBlobStoring();

        builder.Entity<Tenant>(b =>
        {
            // A workspace name is what people sign in to, so two live workspaces never share one: the database backstop
            // for concurrent sign-ups (WorkspaceAppService answers the violation with DuplicateTenantName). Deleted
            // workspaces keep their rows, hence the filter.
            var nameIndex = b.HasIndex(x => x.NormalizedName).IsUnique();
            if (builder.IsUsingPostgreSql() || builder.IsUsingSqlite())
            {
                nameIndex.HasFilter("\"IsDeleted\" = false");
            }
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
            // check (Tracker:DuplicateProjectMember). NULLS NOT DISTINCT (PostgreSQL 15+) covers the host side too.
            b.HasIndex(x => new { x.TenantId, x.ProjectId, x.UserId }).IsUnique().AreNullsDistinct(false);
        });

        builder.Entity<ProjectMemberRate>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "ProjectMemberRates", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            // Payroll looks up the rate in effect at a block's start: the member's latest EffectiveFrom <= StartedAt.
            b.HasIndex(x => new { x.TenantId, x.ProjectId, x.UserId, x.EffectiveFrom });
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
            b.HasIndex(x => new { x.TenantId, x.UserId, x.StartedAt }).IsDescending(false, false, true);
            b.HasIndex(x => new { x.TenantId, x.ProjectId, x.StartedAt }).IsDescending(false, false, true);
            b.HasIndex(x => new { x.TenantId, x.StartedAt }).IsDescending(false, true); // Tenant-wide reports and lists
            // Idempotency, per member: the backstop for concurrent retries across server instances. NULLS NOT DISTINCT
            // (PostgreSQL 15+) keeps it for host users too, whose TenantId is null.
            b.HasIndex(x => new { x.TenantId, x.UserId, x.ClientActivityId }).IsUnique().AreNullsDistinct(false);
            // Lookup only (not unique): a client id already used by another member is refused (ActivityClientIdConflict).
            b.HasIndex(x => new { x.TenantId, x.ClientActivityId });
            b.HasMany(x => x.Screenshots).WithOne().HasForeignKey(x => x.ActivityId).IsRequired();
        });

        builder.Entity<Screenshot>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Screenshots", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.HasIndex(x => new { x.TenantId, x.CapturedAt }).IsDescending(false, true);
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
            // One row per agent install; NULLS NOT DISTINCT (PostgreSQL 15+) keeps that true for host users, whose TenantId is null.
            b.HasIndex(x => new { x.TenantId, x.UserId, x.DeviceId }).IsUnique().AreNullsDistinct(false);
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
        });

        builder.Entity<Dosi.Tracker.Billing.Invoice>(b =>
        {
            b.ToTable(TrackerConsts.DbTablePrefix + "Invoices", TrackerConsts.DbSchema);
            b.ConfigureByConvention();
            b.Property(x => x.Status).IsRequired().HasMaxLength(32);

            // At most one live invoice per tenant and billing period: the database backstop for concurrent sweeps.
            var periodIndex = b.HasIndex(x => new { x.TenantId, x.BillingPeriodStart }).IsUnique();
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
        });
    }
}
