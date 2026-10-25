import DosiTrackerCore
import SwiftUI

// The sign-in window (port of the Windows agent's `ui/login.rs`): email and
// password — the workspace is picked after signing in, from the ones the
// account belongs to — the server address under "Advanced", Enter submits, a
// show/hide password toggle, and the server's own reason when it refuses.
// After "Sign-in expired" the email is prefilled and only the password is
// asked for; when the server demands a new password, the form asks for it.

struct SignInView: View {
    @ObservedObject var model: AppModel

    @State private var email = ""
    @State private var password = ""
    @State private var newPassword = ""
    @State private var confirmPassword = ""
    @State private var server = ""
    @State private var showPassword = false
    @State private var showAdvanced = false
    @State private var prefilled = false
    @State private var localError: String?
    @FocusState private var focus: Field?

    private enum Field: Hashable {
        case email
        case password
    }

    private var snapshot: TrackerSnapshot { model.snapshot }

    private var mustChangePassword: Bool { snapshot.passwordChangeToken != nil }

    private var canSubmit: Bool {
        guard !snapshot.signingIn, !email.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !password.isEmpty
        else {
            return false
        }
        return !mustChangePassword || !newPassword.isEmpty
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 18) {
                header
                if snapshot.status == .signInExpired {
                    Banner(.warning, "Your sign-in has expired. Enter your password to continue — your tracked time is kept.")
                }
                Card {
                    VStack(alignment: .leading, spacing: 12) {
                        accountFields
                        passwordField
                        if mustChangePassword {
                            newPasswordFields
                        }
                        advanced
                        if let error = localError ?? snapshot.loginError {
                            Banner(.danger, error)
                        }
                        submitButton
                    }
                }
                footer
            }
            .padding(24)
            .frame(maxWidth: 420)
            .frame(maxWidth: .infinity)
        }
        .frame(minWidth: 400, idealWidth: 440, minHeight: 520, idealHeight: 600)
        .onAppear(perform: prefill)
        .onChange(of: snapshot.status) { _, status in
            if status == .signInExpired {
                prefilled = false
                prefill()
            }
        }
    }

    // MARK: Parts

    private var header: some View {
        VStack(spacing: 8) {
            LogoMark(size: 52)
            Text("Welcome to Dosi Tracker").font(.title2.weight(.semibold))
            Text("Sign in with your work email to start tracking.")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .padding(.top, 8)
    }

    private var accountFields: some View {
        VStack(alignment: .leading, spacing: 12) {
            field("Email") {
                TextField("", text: $email, prompt: Text("you@company.com"))
                    .textFieldStyle(.roundedBorder)
                    .textContentType(.username)
                    .focused($focus, equals: .email)
                    .onSubmit(submit)
            }
        }
    }

    private var passwordField: some View {
        field(mustChangePassword ? "Current password" : "Password") {
            HStack(spacing: 6) {
                passwordInput
                    .textFieldStyle(.roundedBorder)
                    .textContentType(.password)
                    .focused($focus, equals: .password)
                    .onSubmit(submit)
                Button {
                    showPassword.toggle()
                } label: {
                    Image(systemName: showPassword ? "eye.slash" : "eye")
                }
                .buttonStyle(.borderless)
                .help(showPassword ? "Hide password" : "Show password")
            }
        }
    }

    @ViewBuilder
    private var passwordInput: some View {
        if showPassword {
            TextField("", text: $password, prompt: Text("Your password"))
        } else {
            SecureField("", text: $password, prompt: Text("Your password"))
        }
    }

    private var newPasswordFields: some View {
        VStack(alignment: .leading, spacing: 12) {
            field("New password") {
                SecureField("", text: $newPassword, prompt: Text("Choose a new password"))
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(submit)
            }
            field("Confirm new password") {
                SecureField("", text: $confirmPassword, prompt: Text("Type it again"))
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(submit)
            }
        }
    }

    private var advanced: some View {
        DisclosureGroup("Advanced", isExpanded: $showAdvanced) {
            VStack(alignment: .leading, spacing: 6) {
                field("Server address") {
                    TextField("", text: $server, prompt: Text("https://tracker.example.com"))
                        .textFieldStyle(.roundedBorder)
                        .disabled(snapshot.serverURLIsManaged)
                        .onSubmit(submit)
                }
                Text(serverHint)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.top, 6)
        }
        .font(.callout)
    }

    private var serverHint: String {
        if snapshot.serverURLIsManaged {
            return "Your organisation set the server address."
        }
        return "The address your workspace admin gave you (shown on the web dashboard's Download page)."
    }

    private var submitButton: some View {
        Button(action: submit) {
            HStack {
                if snapshot.signingIn {
                    ProgressView().controlSize(.small)
                }
                Text(snapshot.signingIn ? "Signing in…" : "Sign in")
                    .frame(maxWidth: .infinity)
            }
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .keyboardShortcut(.defaultAction)
        .disabled(!canSubmit)
    }

    private var footer: some View {
        let shown = server.trimmingCharacters(in: .whitespacesAndNewlines)
        let address = Formatting.host(shown.isEmpty || snapshot.serverURLIsManaged ? snapshot.serverURL : shown)
        return VStack(spacing: 4) {
            Text("Your workspace admin decides what is captured.")
            Text("Server: \(address)")
                .foregroundStyle(.tertiary)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    /// A field with its label above it.
    private func field<Content: View>(_ label: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.callout.weight(.semibold)).foregroundStyle(.secondary)
            content()
        }
    }

    /// The last email and server; after "Sign-in expired" only the password
    /// is left to type.
    private func prefill() {
        guard !prefilled else { return }
        prefilled = true
        if email.isEmpty {
            email = snapshot.displayName.isEmpty ? snapshot.suggestedUsername : snapshot.displayName
        }
        if server.isEmpty {
            server = snapshot.suggestedServerURL
        }
        focus = email.isEmpty ? .email : .password
    }

    private func submit() {
        guard canSubmit else { return }
        localError = nil
        if mustChangePassword && newPassword != confirmPassword {
            localError = "The new passwords do not match."
            return
        }
        let typedServer = server.trimmingCharacters(in: .whitespacesAndNewlines)
        model.signIn(
            email: email,
            password: password,
            server: typedServer.isEmpty || snapshot.serverURLIsManaged ? nil : typedServer,
            newPassword: mustChangePassword ? newPassword : nil
        )
        if !mustChangePassword {
            password = ""
        }
    }
}
