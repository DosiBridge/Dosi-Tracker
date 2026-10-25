import Foundation

/// A workspace the signed-in user belongs to (`GET /api/app/workspaces/mine`,
/// cross-agent contract): where the agent tracks, named on every
/// workspace-scoped call by the `X-Workspace-Id` header.
public struct Workspace: Codable, Equatable, Sendable, Identifiable {
    /// The workspace GUID, sent as `X-Workspace-Id`.
    public var id: String
    public var name: String
    /// `owner`, `manager` or `member`; nil from a server that does not say.
    public var role: String?
    public var planName: String?
    public var isOwner: Bool
    /// An archived workspace takes no tracking: it is never offered or chosen.
    public var isArchived: Bool

    public init(id: String, name: String, role: String? = nil, planName: String? = nil, isOwner: Bool = false,
                isArchived: Bool = false) {
        self.id = id
        self.name = name
        self.role = role
        self.planName = planName
        self.isOwner = isOwner
        self.isArchived = isArchived
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, role, planName, isOwner, isArchived
    }

    /// Lenient: only the id is required; a field of an unexpected type reads
    /// as absent.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = (try? c.decodeIfPresent(String.self, forKey: .name)) ?? ""
        role = (try? c.decodeIfPresent(String.self, forKey: .role))?.nonEmptyTrimmed
        planName = (try? c.decodeIfPresent(String.self, forKey: .planName))?.nonEmptyTrimmed
        isOwner = (try? c.decodeIfPresent(Bool.self, forKey: .isOwner)) ?? false
        isArchived = (try? c.decodeIfPresent(Bool.self, forKey: .isArchived)) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(id, forKey: .id)
        try c.encode(name, forKey: .name)
        try c.encodeIfPresent(role, forKey: .role)
        try c.encodeIfPresent(planName, forKey: .planName)
        try c.encode(isOwner, forKey: .isOwner)
        try c.encode(isArchived, forKey: .isArchived)
    }

    /// The name to show: the server's, else a placeholder.
    public var displayName: String {
        name.nonEmptyTrimmed ?? "Unnamed workspace"
    }

    /// "Owner", "Manager", "Member" — the role as the picker shows it.
    public var roleLabel: String? {
        if let role = role {
            return role.prefix(1).uppercased() + role.dropFirst().lowercased()
        }
        return isOwner ? "Owner" : nil
    }

    /// Whether this is the workspace `id` names (GUIDs compare case-insensitively).
    public func hasId(_ other: String?) -> Bool {
        guard let other = other?.nonEmptyTrimmed else { return false }
        return id.caseInsensitiveCompare(other) == .orderedSame
    }

    /// The list the server sent: a plain array, or ABP's `{"items":[…]}`.
    public static func decodeList(from data: Data) throws -> [Workspace] {
        let decoder = JSONDecoder()
        if let list = try? decoder.decode([Workspace].self, from: data) {
            return list
        }
        return try decoder.decode(Page.self, from: data).items
    }

    private struct Page: Decodable {
        let items: [Workspace]
    }
}

/// Which workspace to track in after signing in (cross-agent contract):
/// archived ones are never offered; the one asked for (the workspace tracked
/// before, by id) when it is still listed, else the only one, else the user
/// chooses.
public enum WorkspaceSelection: Equatable, Sendable {
    case selected(Workspace)
    /// Several: the user picks one of these.
    case choose([Workspace])
    /// The user is in no (active) workspace.
    case noWorkspace

    public static func resolve(_ listed: [Workspace], preferred: String?) -> WorkspaceSelection {
        let usable = listed.filter { !$0.isArchived }
        if let wanted = usable.first(where: { $0.hasId(preferred) }) {
            return .selected(wanted)
        }
        switch usable.count {
        case 0:
            return .noWorkspace
        case 1:
            return .selected(usable[0])
        default:
            return .choose(usable)
        }
    }
}
