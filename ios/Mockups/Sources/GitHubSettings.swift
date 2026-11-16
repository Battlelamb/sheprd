import SwiftUI

// Settings > GitHub: one account per host, lent by a paired computer. The
// phone never signs in itself, so there is no sign-in button here.
// A: a GitHub section in Settings, one row per host; state 1 opens a host.
// B: a GitHub page of its own, a section per host with its facts.
// C: account cards, and a host no computer lends shown as a hint.
// D: by computer: what each one lends, and which is in use.

private struct SettingsFrame<Content: View, Pushed: View>: View {
    var title: String? = nil
    /// Shown as pushed on top, with a back button, when there is one.
    var pushed: Pushed?
    @ViewBuilder var content: Content

    init(title: String? = nil, pushed: Pushed? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.pushed = pushed
        self.content = content()
    }

    var body: some View {
        GHTabs(selected: .settings) {
            NavigationStack(path: .constant(pushed == nil ? [] : [1])) {
                List { content }
                    .listSectionSpacing(14)
                    .navigationTitle(title ?? "")
                    .navigationBarTitleDisplayMode(.inline)
                    .navigationDestination(for: Int.self) { _ in pushed }
            }
        }
    }
}

extension SettingsFrame where Pushed == EmptyView {
    init(title: String? = nil, @ViewBuilder content: () -> Content) {
        self.init(title: title, pushed: nil, content: content)
    }
}

private let desktopNote = "Accounts come from your computers. To add one, sign in to GitHub in Sheprd on a computer, or with gh for an Enterprise host."

private struct AccountLine: View {
    var account: GHAccount
    var size: CGFloat = 36
    var showsName = false

    var body: some View {
        HStack(spacing: 12) {
            GHAvatar(person: GHPerson(login: account.login, tint: account.tint), size: size)
            VStack(alignment: .leading, spacing: 2) {
                Text(account.login).lineLimit(1)
                if showsName {
                    Text(account.name).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                } else {
                    Text(account.host).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                        .truncationMode(.middle)
                }
            }
        }
    }
}

private struct LentBy: View {
    var computer: String

    var body: some View {
        GHLabel(computer, systemImage: computer == "studio" ? "macstudio" : "laptopcomputer")
            .font(.caption).foregroundStyle(.secondary).lineLimit(1)
    }
}

// MARK: A

struct GHSettingsRows: View {
    var state: Int

    var body: some View {
        SettingsFrame(pushed: state == 1 ? hostPage : nil) {
            Section("Computers") {
                ForEach(["MacBook Pro", "studio", "build-01"], id: \.self) { name in
                    NavigationLink {} label: { Text(name) }
                }
            }
            Section {
                ForEach(GH.accounts) { account in
                    NavigationLink {} label: {
                        HStack {
                            AccountLine(account: account, size: 32)
                            Spacer(minLength: 8)
                            LentBy(computer: account.lentBy)
                        }
                    }
                }
            } header: {
                Text("GitHub")
            } footer: {
                Text(desktopNote)
            }
            Section("Connection") {
                LabeledContent("Relays", value: "1")
            }
        }
    }

    private var hostPage: some View {
        List {
            Section {
                VStack(spacing: 8) {
                    GHAvatar(person: GH.octocat, size: 72)
                    Text("octocat").font(.title3.weight(.semibold))
                    Text("The Octocat").font(.subheadline).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity)
                .listRowBackground(Color.clear)
            }
            Section {
                LabeledContent("Lent by", value: "MacBook Pro")
                LabeledContent("Sign-in", value: "Sheprd")
                LabeledContent("Token", value: "Renews in 6 h")
                LabeledContent("Also offered by", value: "studio")
            } footer: {
                Text("When MacBook Pro renews its sign-in, this phone asks it again. If it cannot reach GitHub or the computer, answers come from the computer, marked “via MacBook Pro”.")
            }
            Section {
                Button("Forget on This iPhone", role: .destructive) {}
            } footer: {
                Text("To stop lending it, turn off Share GitHub sign-in for this phone in Sheprd's Remote control settings on MacBook Pro.")
            }
        }
        .navigationTitle("github.com")
        .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: B

struct GHSettingsPage: View {
    var state: Int

    var body: some View {
        SettingsFrame(title: "GitHub") {
            Section {
                Label {
                    Text(desktopNote).font(.subheadline).foregroundStyle(.secondary)
                } icon: {
                    Image(systemName: "laptopcomputer.and.iphone").foregroundStyle(.tint)
                }
            }
            ForEach(GH.accounts) { account in
                Section {
                    AccountLine(account: account, size: 44, showsName: true)
                    LabeledContent("Lent by") { LentBy(computer: account.lentBy).font(.body) }
                    LabeledContent("Source", value: account.source)
                    LabeledContent("Expires", value: account.expires ?? "Never")
                } header: {
                    Text(account.host).textCase(nil)
                } footer: {
                    if account.expires == nil {
                        Text("A gh token does not expire. Run gh auth refresh on studio, or revoke it on GitHub, to end it.")
                    }
                }
            }
        }
    }
}

// MARK: C

struct GHSettingsCards: View {
    var state: Int

    var body: some View {
        SettingsFrame(title: "GitHub") {
            Section {
                ForEach(GH.accounts) { account in
                    Card(account: account)
                }
                missing
            } footer: {
                Text(desktopNote)
            }
            .listRowBackground(Color.clear)
            .listRowInsets(EdgeInsets(top: 5, leading: 0, bottom: 5, trailing: 0))
            .listRowSeparator(.hidden)
        }
    }

    private var missing: some View {
        HStack(spacing: 12) {
            Image(systemName: "person.crop.circle.badge.questionmark").font(.title2).foregroundStyle(.secondary)
                .frame(width: 44)
            VStack(alignment: .leading, spacing: 3) {
                Text(verbatim: "git.octo-org.example").font(.subheadline.monospaced()).lineLimit(1)
                Text("A workspace on build-01 uses this host, but no computer lends an account for it. Its pull requests come from build-01.")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(RoundedRectangle(cornerRadius: 22).strokeBorder(.quaternary, style: StrokeStyle(lineWidth: 1, dash: [5, 4])))
    }
}

private struct Card: View {
    var account: GHAccount

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                GHAvatar(person: GHPerson(login: account.login, tint: account.tint), size: 48)
                VStack(alignment: .leading, spacing: 2) {
                    Text(account.login).font(.headline).lineLimit(1)
                    Text(account.name).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 4)
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
            }
            Text(verbatim: account.host).font(.caption.monospaced()).foregroundStyle(.secondary)
            HStack(spacing: 6) {
                GHLabel("from \(account.lentBy)", systemImage: "arrow.down.forward.circle")
                    .padding(.horizontal, 8).padding(.vertical, 3).background(.fill.tertiary, in: .capsule)
                Text(account.expires ?? "gh · no expiry")
                    .padding(.horizontal, 8).padding(.vertical, 3).background(.fill.tertiary, in: .capsule)
            }
            .font(.caption).lineLimit(1)
        }
        .padding(14)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 22))
    }
}

// MARK: D

struct GHSettingsByComputer: View {
    var state: Int

    var body: some View {
        SettingsFrame(title: "GitHub") {
            Section {
                ForEach(GH.accounts) { account in
                    HStack {
                        AccountLine(account: account)
                        Spacer(minLength: 8)
                        Image(systemName: "checkmark").foregroundStyle(.green).font(.subheadline.weight(.semibold))
                    }
                }
            } header: {
                Text("This iPhone reads as")
            }
            Section {
                Lends(host: "github.com", login: "octocat", source: "Sheprd · renews in 6 h", inUse: true)
            } header: {
                Label("MacBook Pro", systemImage: "laptopcomputer").textCase(nil)
            }
            Section {
                Lends(host: "github.acme-corp.com", login: "octocat-acme", source: "gh · no expiry", inUse: true)
                Lends(host: "github.com", login: "octocat", source: "gh · no expiry", inUse: false)
            } header: {
                Label("studio", systemImage: "macstudio").textCase(nil)
            }
            Section {
                Text("Lends nothing").foregroundStyle(.secondary)
            } header: {
                Label("build-01", systemImage: "server.rack").textCase(nil)
            } footer: {
                Text("When two computers lend one host, the one reached most recently is used. \(desktopNote)")
            }
        }
    }
}

private struct Lends: View {
    var host: String
    var login: String
    var source: String
    var inUse: Bool

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: host).lineLimit(1).truncationMode(.middle)
                Text(verbatim: "\(login) · \(source)").font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 4)
            Text(inUse ? "In use" : "Spare").font(.caption.weight(.semibold))
                .foregroundStyle(inUse ? AnyShapeStyle(.green) : AnyShapeStyle(.secondary))
        }
    }
}
