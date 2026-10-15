using System;
using System.Collections.Generic;
using System.ComponentModel.DataAnnotations;
using System.Linq;
using System.Threading.Tasks;
using Dosi.Tracker.Teams;
using Microsoft.AspNetCore.Identity;
using Microsoft.Extensions.Options;
using Volo.Abp;
using Volo.Abp.DependencyInjection;
using Volo.Abp.Guids;
using Volo.Abp.Identity;
using Volo.Abp.Validation;
using IdentityUser = Volo.Abp.Identity.IdentityUser;

namespace Dosi.Tracker.Identity;

/// <summary>Creates the one global account of a person.</summary>
public class TrackerAccountFactory : ITransientDependency
{
    private readonly IdentityUserManager _userManager;
    private readonly IOptions<IdentityOptions> _identityOptions;
    private readonly IGuidGenerator _guidGenerator;

    public TrackerAccountFactory(IdentityUserManager userManager, IOptions<IdentityOptions> identityOptions, IGuidGenerator guidGenerator)
    {
        _userManager = userManager;
        _identityOptions = identityOptions;
        _guidGenerator = guidGenerator;
    }

    public virtual async Task<bool> EmailExistsAsync(string email)
    {
        return await _userManager.FindByEmailAsync(email.Trim()) != null;
    }

    /// <summary>Throws <c>Tracker:EmailAlreadyRegistered</c> when the email has an account.</summary>
    public virtual async Task EnsureEmailIsFreeAsync(string email)
    {
        if (await EmailExistsAsync(email))
        {
            throw new BusinessException(TrackerDomainErrorCodes.EmailAlreadyRegistered).WithData("email", email.Trim());
        }
    }

    /// <summary>A validation error (400) on <paramref name="member"/> for a password the policy refuses.</summary>
    public virtual async Task CheckPasswordAsync(string email, string password, string member)
    {
        await _identityOptions.SetAsync();
        var probe = new IdentityUser(_guidGenerator.Create(), "probe", email);
        var errors = new List<ValidationResult>();
        foreach (var validator in _userManager.PasswordValidators)
        {
            var result = await validator.ValidateAsync(_userManager, probe, password);
            errors.AddRange(result.Errors.Select(e => new ValidationResult(e.Description, new[] { member })));
        }

        if (errors.Count > 0)
        {
            throw new AbpValidationException(errors);
        }
    }

    public virtual async Task<IdentityUser> CreateAsync(string email, string password, string? name, string? surname)
    {
        await EnsureEmailIsFreeAsync(email);
        await _identityOptions.SetAsync();

        var trimmedEmail = email.Trim();
        var user = new IdentityUser(_guidGenerator.Create(), await CreateUserNameAsync(trimmedEmail), trimmedEmail)
        {
            Name = name?.Trim(),
            Surname = surname?.Trim()
        };
        (await _userManager.CreateAsync(user, password)).CheckErrors();
        return user;
    }

    /// <summary>The email itself when Identity accepts it as a user name and nobody uses it; otherwise a name derived
    /// from the address, made unique with a number.</summary>
    protected virtual async Task<string> CreateUserNameAsync(string email)
    {
        var allowed = _userManager.Options.User.AllowedUserNameCharacters;
        if (InviteUserNames.IsAllowed(email, allowed) && await _userManager.FindByNameAsync(email) == null)
        {
            return email;
        }

        var stem = InviteUserNames.Stem(email, allowed);
        for (var attempt = 1; ; attempt++)
        {
            var candidate = InviteUserNames.Candidate(stem, attempt);
            if (await _userManager.FindByNameAsync(candidate) == null)
            {
                return candidate;
            }
        }
    }
}
