using System;
using System.Threading.Tasks;
using Dosi.Tracker.Permissions;
using Microsoft.AspNetCore.Authorization;
using Volo.Abp.Authorization;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Domain.Entities;
using Volo.Abp.Domain.Repositories;
using Volo.Abp.Timing;
using Volo.Abp.Users;

namespace Dosi.Tracker.Activities;

/// <summary>
/// Who may change the task note of a block (<c>PUT /api/app/activity/{id}/note</c>), shared by that endpoint and by AI
/// note suggestions, so a suggestion is only ever offered to someone who could save it.
/// <para>
/// The member who tracked a block may change its note within <see cref="NoteEditWindow"/> of its start; team managers
/// (Tracker.Team.Manage) may change any note at any time. Someone else's block is hidden entirely (404) from callers
/// who cannot read it (no Tracker.Activities.ViewAll), and refused (403) to those who can read but not manage.
/// </para>
/// </summary>
public class ActivityNoteRules : ITransientDependency
{
    /// <summary>How long after its start the owner of a block may still change its note.</summary>
    public static readonly TimeSpan NoteEditWindow = TimeSpan.FromDays(30);

    private readonly IRepository<Activity, Guid> _activityRepository;
    private readonly IAuthorizationService _authorizationService;
    private readonly ICurrentUser _currentUser;
    private readonly IClock _clock;

    public ActivityNoteRules(
        IRepository<Activity, Guid> activityRepository,
        IAuthorizationService authorizationService,
        ICurrentUser currentUser,
        IClock clock)
    {
        _activityRepository = activityRepository;
        _authorizationService = authorizationService;
        _currentUser = currentUser;
        _clock = clock;
    }

    /// <summary>Throws unless the caller may change this block's note: <see cref="AbpAuthorizationException"/> (403)
    /// when they may not, <see cref="EntityNotFoundException"/> (404) when they may not even read the block.</summary>
    public virtual async Task EnsureCanEditNoteAsync(Activity activity)
    {
        if (activity.UserId == _currentUser.GetId())
        {
            var age = AgentContextNormalizer.AsUtc(_clock.Now) - AgentContextNormalizer.AsUtc(activity.StartedAt);
            if (age > NoteEditWindow && !await _authorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage))
            {
                throw new AbpAuthorizationException(
                    $"The note of a block can be changed within {NoteEditWindow.TotalDays:0} days of its start.");
            }
        }
        else if (!await _authorizationService.IsGrantedAsync(TrackerPermissions.Team.Manage))
        {
            if (!await _authorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
            {
                // Other members' blocks are hidden entirely from callers who cannot read them.
                throw new EntityNotFoundException(typeof(Activity), activity.Id);
            }

            throw new AbpAuthorizationException("Only the member who tracked a block, or a team manager, can change its note.");
        }
    }

    /// <summary>
    /// The block a note suggestion is made for: one whose note the caller may change (<see cref="EnsureCanEditNoteAsync"/>)
    /// and, when it is someone else's, may also read (Tracker.Activities.ViewAll). A team manager without ViewAll may
    /// save another member's note yet never sees that member's blocks, so their content is not read for them (404).
    /// </summary>
    public virtual async Task<Activity> LoadForSuggestionAsync(Guid activityId)
    {
        var activity = await _activityRepository.GetAsync(activityId);

        await EnsureCanEditNoteAsync(activity);

        if (activity.UserId != _currentUser.GetId() &&
            !await _authorizationService.IsGrantedAsync(TrackerPermissions.Activities.ViewAll))
        {
            throw new EntityNotFoundException(typeof(Activity), activityId);
        }

        return activity;
    }
}
