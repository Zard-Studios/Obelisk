import SwiftUI

struct RootView: View {
    @Bindable var model: AppModel
    @Environment(\.colorScheme) private var colorScheme
    @State private var friendsAvatars = FriendsAvatarCache()
    @State private var shownFriends: FriendsPresentationState?
    @State private var windowSize = CGSize(width: 1120, height: 760)

    var body: some View {
        NavigationSplitView {
            SidebarView(model: model)
                .navigationSplitViewColumnWidth(ObeliskStyle.sidebarWidth)
        } detail: {
            Group {
                switch model.selection {
                case .home: HomeView(model: model)
                case .steam: SteamView(model: model.steam)
                case .epic: EpicView(model: model.epic)
                case .gog: GogView(model: model.gog)
                case .settings: SettingsView(model: model)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(ObeliskPalette(colorScheme: colorScheme).background)
        }
        .frame(minWidth: 860, minHeight: 600)
        .tint(ObeliskStyle.accent)
        .onGeometryChange(for: CGSize.self) { $0.size } action: { windowSize = $0 }
        .sheet(isPresented: Binding(get: { model.presentedFriends != nil }, set: { if !$0 { model.closeFriends() } })) {
            // The last friends shown, while the sheet goes away.
            if let friends = model.presentedFriends ?? shownFriends {
                FriendsModal(state: friends, avatars: friendsAvatars,
                             chat: friends.store.id == "steam" ? model.steam.chat : nil,
                             dismiss: { model.closeFriends() },
                             retry: model.retryPresentedFriends, signIn: model.signInPresentedFriends)
                    .frame(width: 520, height: 550)
                    .id(friends.account)
            }
        }
        .sheet(item: $model.presentedStore) { store in
            // As large as the window leaves room for: a sheet does not keep
            // within it.
            StorefrontStoreModal(store: store) { model.presentedStore = nil }
                .frame(width: min(1100, max(640, windowSize.width - 120)),
                       height: max(460, windowSize.height - 50))
        }
        .environment(\.presentStore) { model.presentedStore = $0 }
        .sheet(item: $model.bugReport) { report in
            BugReportSheet(model: report) { model.bugReport = nil }
        }
        .environment(\.reportBug) { model.reportBug($0) }
        .environment(model.estimates)
        .environment(model.steam.libraryArtwork)
        // A store belongs to the library it opened from.
        .onChange(of: model.selection) { model.presentedStore = nil }
        .onChange(of: model.steam.isFriendsPresented, initial: true) { _, presented in
            if presented { model.presentFriends(storeID: "steam") }
            else { model.closeFriends(storeID: "steam") }
        }
        .onChange(of: model.epic.isFriendsPresented, initial: true) { _, presented in
            if presented { model.presentFriends(storeID: "epic") }
            else { model.closeFriends(storeID: "epic") }
        }
        .onChange(of: model.steam.friends?.account) { _, _ in model.validateFriendsAccount() }
        .onChange(of: model.epic.friends?.account) { _, _ in model.validateFriendsAccount() }
        .onChange(of: model.presentedFriends?.account, initial: true) { _, account in
            friendsAvatars.activate(account: account)
            if let friends = model.presentedFriends { shownFriends = friends }
        }
        .sheet(isPresented: Binding(get: { model.presentedPrompt != nil }, set: { _ in })) {
            prompt.interactiveDismissDisabled()
        }
        .installPlacement(Bindable(model.gamesFolder).placement)
    }

    @ViewBuilder private var prompt: some View {
        switch model.presentedPrompt {
        case .steamVoile:
            voileUpdate(model.steam.gameSessions)
        case .steamRosetta:
            RosettaPromptView(model: model.steam.rosetta)
        case .steamCloud:
            CloudQuestionSheet(model: model.steam)
        case .steamWindowsSignIn:
            if let signIn = model.steam.installations.windowsSteamSignIn {
                WindowsSteamSignInSheet(model: signIn.model, gameName: signIn.receipt.name,
                                        finished: { model.steam.installations.finishWindowsSteamSignIn(signedIn: true) },
                                        closed: { model.steam.installations.finishWindowsSteamSignIn(signedIn: false) })
            }
        case .epicRosetta:
            RosettaPromptView(model: model.epic.rosetta, storefront: "epic")
        case .epicVoile:
            voileUpdate(model.epic.gameSessions)
        case .epicCloud:
            CloudQuestionSheet(model: model.epic)
        case .gogVoile:
            voileUpdate(model.gog.gameSessions)
        case .gogRosetta:
            RosettaPromptView(model: model.gog.rosetta, storefront: "gog")
        case nil:
            EmptyView()
        }
    }

    @ViewBuilder private func voileUpdate<Store>(_ sessions: GameSessions<Store>) -> some View {
        if let offer = sessions.voileUpdate {
            VoileUpdateSheet(offer: offer, answer: sessions.answerVoileUpdate)
        }
    }
}

private extension View {
    /// Asks the player which folder a game installs in, with each one's free
    /// space, when several connected ones hold games.
    func installPlacement(_ placement: Binding<InstallPlacement?>) -> some View {
        let shown = placement.wrappedValue
        return confirmationDialog(L10n.string("ui.games_folder.install_where", shown?.gameName ?? ""),
                                  isPresented: Binding(get: { placement.wrappedValue != nil },
                                                       set: { if !$0 { placement.wrappedValue = nil } }),
                                  titleVisibility: .visible, presenting: shown) { shown in
            ForEach(shown.locations) { location in
                Button(location.availableBytes.map {
                    L10n.string("ui.games_folder.install_location", location.title,
                                ByteCountFormatter.string(fromByteCount: Int64(clamping: $0), countStyle: .file))
                } ?? location.title) {
                    shown.install(location.path)
                }
            }
            Button(L10n.string("ui.common.cancel"), role: .cancel) {}
        } message: { _ in
            Text(L10n.string("ui.games_folder.install_where_message"))
        }
    }
}
