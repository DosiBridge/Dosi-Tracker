using System.Net;
using Localization.Resources.AbpUi;
using Dosi.Tracker.Localization;
using Volo.Abp.Account;
using Volo.Abp.AspNetCore.ExceptionHandling;
using Volo.Abp.SettingManagement;
using Volo.Abp.FeatureManagement;
using Volo.Abp.Identity;
using Volo.Abp.Modularity;
using Volo.Abp.PermissionManagement.HttpApi;
using Volo.Abp.Localization;

namespace Dosi.Tracker;

 [DependsOn(
    typeof(TrackerApplicationContractsModule),
    typeof(AbpPermissionManagementHttpApiModule),
    typeof(AbpSettingManagementHttpApiModule),
    typeof(AbpAccountHttpApiModule),
    typeof(AbpIdentityHttpApiModule),
    typeof(AbpFeatureManagementHttpApiModule)
    )]
public class TrackerHttpApiModule : AbpModule
{
    /// <summary>425 Too Early: System.Net.HttpStatusCode has no named member for it.</summary>
    private const HttpStatusCode TooEarly = (HttpStatusCode)425;

    public override void ConfigureServices(ServiceConfigurationContext context)
    {
        ConfigureLocalization();
        ConfigureErrorCodeHttpStatuses();
    }

    private void ConfigureLocalization()
    {
        Configure<AbpLocalizationOptions>(options =>
        {
            options.Resources
                .Get<TrackerResource>()
                .AddBaseTypes(
                    typeof(AbpUiResource)
                );
        });
    }

    /// <summary>
    /// HTTP status of every Tracker business error (shared contract §1). ABP answers an unmapped
    /// BusinessException with 403, which would make temporary or fixable rejections look like a missing
    /// permission; the desktop agents decide between retrying, refreshing and parking a block by these statuses.
    /// </summary>
    private void ConfigureErrorCodeHttpStatuses()
    {
        Configure<AbpExceptionHttpStatusCodeOptions>(options =>
        {
            options.Map(TrackerDomainErrorCodes.ActivityTimeRangeInvalid, HttpStatusCode.UnprocessableEntity);
            options.Map(TrackerDomainErrorCodes.ActivityClientIdRequired, HttpStatusCode.BadRequest);
            options.Map(TrackerDomainErrorCodes.ActivityClientIdConflict, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.ActivityInFuture, TooEarly);
            options.Map(TrackerDomainErrorCodes.ActivityTooLong, HttpStatusCode.UnprocessableEntity);
            options.Map(TrackerDomainErrorCodes.ActivityTooOld, HttpStatusCode.UnprocessableEntity);
            options.Map(TrackerDomainErrorCodes.ActivityOverlaps, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.ProjectArchived, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.NotProjectMember, HttpStatusCode.Forbidden);
            options.Map(TrackerDomainErrorCodes.InvalidCaptureData, HttpStatusCode.BadRequest);
            options.Map(TrackerDomainErrorCodes.CaptureTooLarge, HttpStatusCode.RequestEntityTooLarge);
            options.Map(TrackerDomainErrorCodes.SubscriptionInactive, HttpStatusCode.PaymentRequired);
            options.Map(TrackerDomainErrorCodes.SeatLimitReached, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.DuplicateProjectMember, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.PlanNotFound, HttpStatusCode.NotFound);
            options.Map(TrackerDomainErrorCodes.PlanInUse, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.ReportRangeInvalid, HttpStatusCode.BadRequest);
            options.Map(TrackerDomainErrorCodes.InvalidUser, HttpStatusCode.NotFound);
            options.Map(TrackerDomainErrorCodes.CannotModifySelf, HttpStatusCode.Forbidden);
            options.Map(TrackerDomainErrorCodes.RoleNotAllowed, HttpStatusCode.Forbidden);
            options.Map(TrackerDomainErrorCodes.WorkspaceRequired, HttpStatusCode.BadRequest);
            options.Map(TrackerDomainErrorCodes.NotWorkspaceMember, HttpStatusCode.Forbidden);
            options.Map(TrackerDomainErrorCodes.EmailAlreadyRegistered, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.DuplicateWorkspaceName, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.LastOwnerCannotLeave, HttpStatusCode.Conflict);
            options.Map(TrackerDomainErrorCodes.InvitationNotFound, HttpStatusCode.NotFound);
            options.Map(TrackerDomainErrorCodes.InvitationExpired, HttpStatusCode.Gone);
            options.Map(TrackerDomainErrorCodes.InvitationEmailMismatch, HttpStatusCode.Forbidden);
            options.Map(TrackerDomainErrorCodes.AlreadyWorkspaceMember, HttpStatusCode.Conflict);
        });
    }
}
