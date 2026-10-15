using System;
using System.Security.Cryptography;
using System.Text;
using Volo.Abp.Domain.Entities.Auditing;

namespace Dosi.Tracker.Workspaces;

/// <summary>An invitation to join a workspace, sent to an email address that has no account yet. Only the hash of
/// its token is stored.</summary>
public class WorkspaceInvitation : CreationAuditedAggregateRoot<Guid>
{
    public Guid WorkspaceId { get; private set; }
    public string Email { get; private set; } = null!;
    public string NormalizedEmail { get; private set; } = null!;

    /// <summary>The workspace role the invitee gets (<see cref="WorkspaceRoles"/>).</summary>
    public string Role { get; private set; } = null!;

    public Guid? ProjectId { get; private set; }
    public string? ProjectRole { get; private set; }
    public decimal? HourlyRate { get; private set; }

    public string TokenHash { get; private set; } = null!;
    public DateTime ExpiresAt { get; private set; }
    public DateTime? AcceptedAt { get; private set; }
    public Guid? AcceptedUserId { get; private set; }

    protected WorkspaceInvitation()
    {
    }

    public WorkspaceInvitation(
        Guid id,
        Guid workspaceId,
        string email,
        string role,
        Guid? projectId,
        string? projectRole,
        decimal? hourlyRate,
        string tokenHash,
        DateTime expiresAt)
        : base(id)
    {
        WorkspaceId = workspaceId;
        Email = email.Trim();
        NormalizedEmail = NormalizeEmail(Email);
        Role = role;
        ProjectId = projectId;
        ProjectRole = projectRole;
        HourlyRate = hourlyRate;
        TokenHash = tokenHash;
        ExpiresAt = expiresAt;
    }

    public bool IsUsable(DateTime now) => AcceptedAt == null && ExpiresAt > now;

    public bool IsFor(string? email) => email != null && NormalizeEmail(email) == NormalizedEmail;

    public void MarkAccepted(Guid userId, DateTime now)
    {
        AcceptedAt = now;
        AcceptedUserId = userId;
    }

    public static string NormalizeEmail(string email) => email.Trim().ToUpperInvariant();

    public static string NewToken() => Convert.ToHexString(RandomNumberGenerator.GetBytes(32)).ToLowerInvariant();

    public static string HashToken(string token) =>
        Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(token.Trim()))).ToLowerInvariant();
}
