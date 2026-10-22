import XCTest
@testable import Obelisk

/// What a running Steam game asks of Obelisk's overlay through its Steam
/// engine: its overlay's pages, friends' invitations into it and purchases.
final class GameOverlaySteamTests: XCTestCase {
    private static let peak = SteamGame(appId: 3_527_290, name: "PEAK", iconHash: "", lastPlayed: 0, playtimeMinutes: 0)
    private static let peakLobby: UInt64 = 109_775_244_421_126_053
    private static let alice = ChatBridgeTestSession.friendID
    private static let stranger: UInt64 = 76_561_198_000_000_009
    private static let other: UInt64 = 76_561_198_000_000_010
    private static let account = FriendsWireFixture.accountID
    private static let game = OverlayApplication(pid: 4_242, bundleIdentifier: "com.zagrios.obelisk.game.peak")

    func testEverySteamOverlayKindShowsItsPage() {
        func page(_ kind: SteamOverlayKind) -> OverlayRequest? {
            OverlayRequest.steam(kind, appId: 620, account: Self.account)
        }
        let profile = URL(string: "https://steamcommunity.com/profiles/\(Self.stranger)")!

        for dialog in ["friends", "Players", "community", "chatroomgroup/123456"] {
            XCTAssertEqual(page(.dialog(dialog: dialog)), .friends, dialog)
        }
        XCTAssertEqual(page(.dialog(dialog: "achievements")), .achievements)
        XCTAssertEqual(page(.dialog(dialog: "stats")), .achievements)
        XCTAssertEqual(page(.dialog(dialog: "officialgamegroup")),
                       .webPage(URL(string: "https://steamcommunity.com/app/620")!), "The game's community hub")
        XCTAssertEqual(page(.dialog(dialog: "settings")), .home)
        XCTAssertEqual(page(.dialog(dialog: "something-new")), .home)

        XCTAssertEqual(page(.user(dialog: "steamid", steamId: Self.stranger)), .user(steamId: Self.stranger))
        XCTAssertEqual(page(.user(dialog: "chat", steamId: Self.stranger)), .user(steamId: Self.stranger))
        XCTAssertEqual(page(.user(dialog: "achievements", steamId: Self.account)), .achievements, "The account's own")
        XCTAssertEqual(page(.user(dialog: "stats", steamId: Self.stranger)),
                       .webPage(URL(string: "https://steamcommunity.com/profiles/\(Self.stranger)/stats/620/achievements/")!))
        for dialog in ["friendadd", "friendremove", "friendrequestaccept", "friendrequestignore", "jointrade"] {
            XCTAssertEqual(page(.user(dialog: dialog, steamId: Self.stranger)), .webPage(profile), dialog)
        }

        XCTAssertEqual(page(.webPage(url: "https://example.com/news?id=1", modal: true)),
                       .webPage(URL(string: "https://example.com/news?id=1")!))
        XCTAssertEqual(page(.webPage(url: "http://example.com", modal: false)), .webPage(URL(string: "http://example.com")!))
        XCTAssertNil(page(.webPage(url: "file:///etc/passwd", modal: false)), "Only the web")
        XCTAssertNil(page(.webPage(url: "javascript:alert(1)", modal: false)))
        XCTAssertNil(page(.webPage(url: "not a url", modal: false)))

        let store = URL(string: "https://store.steampowered.com/app/620")!
        XCTAssertEqual(page(.store(appId: 620, flag: .showPage)), .webPage(store))
        XCTAssertEqual(page(.store(appId: 620, flag: .addToCart)), .webPage(store))
        XCTAssertEqual(page(.store(appId: 0, flag: .addToCartAndShow)),
                       .webPage(URL(string: "https://store.steampowered.com/")!), "App 0 is the store's front page")

        XCTAssertNil(page(.lobbyInvite(lobbyId: 1)), "The friends list picks for invite dialogs")
        XCTAssertNil(page(.connectInvite(connect: "+connect 1")))
        XCTAssertEqual(page(.remotePlayTogetherInvite(lobbyId: nil)), .friends)
        XCTAssertEqual(page(.remotePlayTogetherInvite(lobbyId: 3)), .friends)
        XCTAssertEqual(page(.remotePlayTogether), .friends)
    }

    func testTheStoreSearchKeepsAPlusApartFromASpace() {
        XCTAssertEqual(GameOverlayController.storeSearchURL("C++ Tycoon").absoluteString,
                       "https://store.steampowered.com/search/?term=C%2B%2B%20Tycoon")
    }

    @MainActor
    func testTheGamesOverlayRequestsOpenTheirPageOncePerNewRequest() async throws {
        let (fixture, model, sessions) = try await signedIn()
        defer { model.signOut() }
        let overlay = fixture.overlay
        // A session first seen with a request, such as one recovered, opens nothing.
        sessions.fake.show(peak(.init(sequence: 3, kind: .dialog(dialog: "achievements"))))
        try await eventually { overlay.session != nil && sessions.active?.overlayRequest?.sequence == 3 }
        await drainMainQueue(times: 3)
        XCTAssertFalse(overlay.isPresented)

        sessions.fake.show(peak(.init(sequence: 4, kind: .dialog(dialog: "achievements"))))
        try await eventually { overlay.isPresented }
        XCTAssertEqual(overlay.request, .achievements)
        try await eventually { sessions.fake.overlayStates.count == 1 }
        XCTAssertEqual(sessions.fake.overlayStates, [.init(session: "peak", active: true, userInitiated: false)],
                       "The game opened it")

        // The same request seen again opens nothing more.
        overlay.hide()
        sessions.fake.show(peak(.init(sequence: 4, kind: .dialog(dialog: "achievements")), lobby: 1))
        try await eventually { sessions.active?.lobbyId == 1 }
        await drainMainQueue(times: 3)
        XCTAssertFalse(overlay.isPresented)
        try await eventually { sessions.fake.overlayStates.count == 2 }
        XCTAssertEqual(sessions.fake.overlayStates.last, .init(session: "peak", active: false, userInitiated: false))

        // A page Obelisk cannot show opens nothing, and the next request still opens.
        sessions.fake.show(peak(.init(sequence: 5, kind: .webPage(url: "file:///etc/hosts", modal: false))))
        try await eventually { sessions.active?.overlayRequest?.sequence == 5 }
        await drainMainQueue(times: 3)
        XCTAssertFalse(overlay.isPresented)
        sessions.fake.show(peak(.init(sequence: 6, kind: .store(appId: 0, flag: .addToCart))))
        try await eventually { overlay.isPresented }
        XCTAssertEqual(overlay.request, .webPage(URL(string: "https://store.steampowered.com/")!))

        // The open overlay turns to the next request: a friend's conversation.
        sessions.fake.show(peak(.init(sequence: 7, kind: .user(dialog: "chat", steamId: Self.alice))))
        try await eventually { overlay.request == .friends }
        XCTAssertEqual(overlay.focus.conversation?.friendID, String(Self.alice))
        XCTAssertEqual(fixture.presenter.shown.count, 2)

        // An invite dialog goes to the friends list's picker.
        var invite = peak(.init(sequence: 8, kind: .lobbyInvite(lobbyId: Self.peakLobby)))
        invite.inviteRequest = SteamInviteRequest(sequence: 8, invite: .lobby(lobbyId: Self.peakLobby))
        sessions.fake.show(invite)
        try await eventually { overlay.invitePicker != nil }
        XCTAssertEqual(overlay.invitePicker?.invite, .lobby(lobbyId: Self.peakLobby))

        // Another session of the game starts from its own first request.
        overlay.hide()
        sessions.fake.show(peak(.init(sequence: 2, kind: .dialog(dialog: "friends")), id: "peak-again"))
        try await eventually { overlay.session?.id == "peak-again" }
        await drainMainQueue(times: 3)
        XCTAssertFalse(overlay.isPresented)
        sessions.fake.show(peak(.init(sequence: 3, kind: .remotePlayTogether), id: "peak-again"))
        try await eventually { overlay.isPresented }
        XCTAssertEqual(overlay.request, .friends)
    }

    @MainActor
    func testTheSteamGameLearnsWhoOpenedTheOverlayAndNothingOnceItEnded() async throws {
        let fixture = OverlayFixture()
        let overlay = fixture.overlay
        let fake = fixture.steamSessions.fake
        fixture.frontmost = Self.game
        fake.show(peak(nil))
        try await eventually { overlay.session != nil && fixture.hotKey.isRegistered }

        // Shift-Tab: the player opened it, and closed it.
        fixture.hotKey.press()
        fixture.hotKey.press()
        try await eventually { fake.overlayStates.count == 2 }
        XCTAssertEqual(fake.overlayStates, [.init(session: "peak", active: true, userInitiated: true),
                                            .init(session: "peak", active: false, userInitiated: true)])

        // The game's request, closed by the player.
        overlay.present(.achievements)
        overlay.hide()
        try await eventually { fake.overlayStates.count == 4 }
        XCTAssertEqual(fake.overlayStates.suffix(2), [.init(session: "peak", active: true, userInitiated: false),
                                                       .init(session: "peak", active: false, userInitiated: false)])

        // A session that refuses changes nothing.
        fake.failSteamCalls("backend.steam.runtime.invalidSession")
        fixture.hotKey.press()
        try await eventually { fake.overlayStates.count == 5 }
        XCTAssertTrue(overlay.isPresented)
        fake.failSteamCalls(nil)

        // The game ends with the overlay open: nothing more is told.
        fake.show(nil)
        try await eventually { overlay.session == nil }
        XCTAssertFalse(overlay.isPresented)
        await drainMainQueue(times: 5)
        XCTAssertEqual(fake.overlayStates.count, 5)

        // Another storefront's game is told nothing.
        fixture.epicSessions.fake.show(.epic(account: "epic-account", id: "epic"))
        try await eventually { overlay.session?.id == "epic" }
        fixture.hotKey.press()
        overlay.hide()
        await drainMainQueue(times: 5)
        XCTAssertEqual(fake.overlayStates.count, 5)
    }

    @MainActor
    func testFriendsInvitationsShowJoinAndDismissAndANewOneToastsOnce() async throws {
        let (fixture, model, sessions) = try await signedIn()
        defer { model.signOut() }
        let overlay = fixture.overlay
        let toaster = fixture.toaster
        let now = UInt64(Date().timeIntervalSince1970)
        let peakApp = Self.peak.appId
        let old = SteamReceivedInvite(friendSteamId: Self.stranger, appId: peakApp, invite: .connect(connect: "+connect 10.0.0.1"),
                                      receivedAt: now - 600)
        let alice = SteamReceivedInvite(friendSteamId: Self.alice, appId: peakApp, invite: .lobby(lobbyId: Self.peakLobby),
                                        receivedAt: now - 500)
        let later = SteamReceivedInvite(friendSteamId: Self.other, appId: peakApp, invite: .lobby(lobbyId: 7),
                                        receivedAt: now - 400)

        // Invitations the session showed when first seen are listed, not toasted.
        sessions.fake.show(peak(nil, invites: [old]))
        try await eventually { overlay.receivedInvites == [old] }
        XCTAssertTrue(toaster.shown.isEmpty)

        // A new one toasts once, over the game, with who sent it.
        sessions.fake.show(peak(nil, invites: [alice, old]))
        try await eventually { toaster.shown.count == 1 }
        XCTAssertEqual(toaster.toasts.first?.message, L10n.string("ui.overlay.invites.toast", "Alice", "PEAK"))
        XCTAssertEqual(toaster.toasts.first?.hint, L10n.string("ui.overlay.invites.toastHint"))
        XCTAssertEqual(toaster.toasts.first?.friend?.steamID, Self.alice)
        XCTAssertEqual(toaster.shown.first?.game, Self.game)
        XCTAssertEqual(overlay.receivedInvites, [alice, old], "Newest first")
        sessions.fake.show(peak(nil, invites: [alice, old], lobby: 1))
        try await eventually { sessions.active?.lobbyId == 1 }
        await drainMainQueue(times: 3)
        XCTAssertEqual(toaster.shown.count, 1, "The same invitation again toasts nothing")

        // A click on it opens the overlay, at the player's demand.
        toaster.click()
        XCTAssertTrue(overlay.isPresented)
        XCTAssertEqual(overlay.request, .home)
        try await eventually { sessions.fake.overlayStates.first?.userInitiated == true }

        // While the overlay shows, a new invitation only joins the list.
        sessions.fake.show(peak(nil, invites: [later, alice, old], lobby: 1))
        try await eventually { overlay.receivedInvites.count == 3 }
        XCTAssertEqual(toaster.shown.count, 1)

        // Dismissed until its friend sends it again.
        overlay.dismiss(old)
        XCTAssertEqual(overlay.receivedInvites, [later, alice])
        let resent = SteamReceivedInvite(friendSteamId: old.friendSteamId, appId: peakApp, invite: old.invite,
                                         receivedAt: now - 300)
        sessions.fake.show(peak(nil, invites: [resent, later, alice], lobby: 1))
        try await eventually { overlay.receivedInvites == [resent, later, alice] }

        // Joining tells the game, which drops it, and the player is back in the game.
        let gate = GameSessionGate<Void>()
        sessions.fake.holdSteamAnswers(gate)
        overlay.accept(alice)
        try await eventually { model.invitations.accepting == SteamReceivedInvitations.Key(alice) }
        XCTAssertTrue(overlay.isPresented, "Until the game is told")
        await gate.resolve(())
        sessions.fake.holdSteamAnswers(nil)
        try await eventually { !overlay.isPresented }
        XCTAssertEqual(sessions.fake.acceptedInvites, [.init(session: "peak", friend: Self.alice,
                                                             invite: .lobby(lobbyId: Self.peakLobby))])
        XCTAssertEqual(overlay.receivedInvites, [resent, later])
        XCTAssertEqual(fixture.presenter.hidden.last, Self.game)

        // A refused join stays on its row, the overlay open.
        overlay.present(.home, userInitiated: true)
        sessions.fake.failSteamCalls("backend.steam.runtime.invalidSession")
        overlay.accept(later)
        try await eventually { model.invitations.failures[SteamReceivedInvitations.Key(later)] != nil }
        XCTAssertEqual(model.invitations.failures[SteamReceivedInvitations.Key(later)],
                       L10n.string("backend.steam.runtime.invalidSession"))
        XCTAssertNil(model.invitations.accepting)
        XCTAssertTrue(overlay.isPresented)
        sessions.fake.failSteamCalls(nil)

        // The game ends: its invitations and its toast go with it.
        sessions.fake.show(nil)
        try await eventually { overlay.receivedInvites.isEmpty && overlay.session == nil }
        XCTAssertFalse(toaster.isShowing)
    }

    @MainActor
    func testAPurchaseAsksThePlayerUntilAnsweredEvenAcrossClosings() async throws {
        let (fixture, model, sessions) = try await signedIn()
        defer { model.signOut() }
        let overlay = fixture.overlay
        let purchase = model.purchase
        sessions.fake.show(peak(nil))
        try await eventually { overlay.session != nil }

        sessions.fake.show(peak(nil, microTxn: SteamMicroTxn(orderId: 77, appId: Self.peak.appId)))
        try await eventually { overlay.isPresented }
        XCTAssertEqual(overlay.request, .microTxn(orderId: 77))
        XCTAssertEqual(overlay.pendingPurchase?.orderId, 77)
        XCTAssertFalse(overlay.back(), "No page behind it")

        // Closed unanswered, it still waits, and shows at the next opening.
        overlay.hide()
        XCTAssertEqual(purchase.pending?.orderId, 77)
        fixture.hotKey.press()
        XCTAssertTrue(overlay.isPresented)
        XCTAssertEqual(overlay.request, .microTxn(orderId: 77))
        overlay.show(.home)
        XCTAssertEqual(overlay.request, .microTxn(orderId: 77), "Home waits for the answer")

        // A refused answer shows its error and keeps the page.
        sessions.fake.failSteamCalls("backend.steam.runtime.invalidSession")
        overlay.answerPurchase(approve: false)
        try await eventually { purchase.error != nil }
        XCTAssertEqual(purchase.error, L10n.string("backend.steam.runtime.invalidSession"))
        XCTAssertFalse(purchase.isAnswering)
        XCTAssertTrue(overlay.isPresented)
        sessions.fake.failSteamCalls(nil)

        // Denied: the game is told, and the player is back in it.
        let gate = GameSessionGate<Void>()
        sessions.fake.holdSteamAnswers(gate)
        overlay.answerPurchase(approve: false)
        try await eventually { purchase.isAnswering }
        XCTAssertNil(purchase.error)
        await gate.resolve(())
        sessions.fake.holdSteamAnswers(nil)
        try await eventually { !overlay.isPresented }
        XCTAssertEqual(sessions.fake.microTxnAnswers, [.init(session: "peak", orderID: 77, approve: false)])
        try await eventually { purchase.pending == nil }
        XCTAssertFalse(purchase.isAnswering)
        fixture.hotKey.press()
        XCTAssertEqual(overlay.request, .home, "Nothing waits any more")
        overlay.hide()

        // The next purchase, authorized.
        sessions.fake.show(peak(nil, microTxn: SteamMicroTxn(orderId: 78, appId: Self.peak.appId)))
        try await eventually { overlay.request == .microTxn(orderId: 78) }
        overlay.answerPurchase(approve: true)
        try await eventually { !overlay.isPresented }
        XCTAssertEqual(sessions.fake.microTxnAnswers.last, .init(session: "peak", orderID: 78, approve: true))

        // A purchase the game stops waiting on leaves its page for home.
        sessions.fake.show(peak(nil, microTxn: SteamMicroTxn(orderId: 79, appId: Self.peak.appId)))
        try await eventually { overlay.request == .microTxn(orderId: 79) }
        sessions.fake.show(peak(nil, lobby: 1))
        try await eventually { purchase.pending == nil }
        XCTAssertTrue(overlay.back())
        XCTAssertEqual(overlay.request, .home)
    }

    @MainActor
    func testAPurchaseWaitingWhenTheSessionIsFirstSeenShowsOnlyOnceTheOverlayOpens() async throws {
        let (fixture, model, sessions) = try await signedIn()
        defer { model.signOut() }
        let overlay = fixture.overlay
        sessions.fake.show(peak(nil, microTxn: SteamMicroTxn(orderId: 5, appId: Self.peak.appId)))
        try await eventually { model.purchase.pending?.orderId == 5 }
        await drainMainQueue(times: 3)
        XCTAssertFalse(overlay.isPresented)
        fixture.hotKey.press()
        XCTAssertEqual(overlay.request, .microTxn(orderId: 5))
    }

    @MainActor
    func testSteamPagesAreSignedInAsTheAccount() async throws {
        let session = ChatBridgeTestSession(games: [Self.peak])
        let requests = WebSignInRequests()
        let model = friendsTestModel(connect: { _ in session }, webCookies: { asked in
            requests.record(asked === session ? Self.account : 0)
            return [SteamWebCookie(domain: "store.steampowered.com", name: "steamLoginSecure",
                                   value: "\(Self.account)%7C%7Caccess", httpOnly: true)]
        })
        let fixture = OverlayFixture(steam: model)
        let store = URL(string: "https://store.steampowered.com/app/620")!
        let noCookies = await fixture.overlay.webCookies(for: store)
        XCTAssertTrue(noCookies.isEmpty, "Signed out")

        await model.start()
        try await eventually { model.friends?.connection == .connected }
        let cookies = await fixture.overlay.webCookies(for: store)
        XCTAssertEqual(cookies.map(\.name), ["steamLoginSecure"])
        XCTAssertEqual(cookies.first?.domain, "store.steampowered.com")
        XCTAssertEqual(cookies.first?.value, "\(Self.account)%7C%7Caccess")
        XCTAssertEqual(cookies.first?.isSecure, true)
        XCTAssertEqual(cookies.first?.isHTTPOnly, true)
        let other = await fixture.overlay.webCookies(for: URL(string: "https://store.epicgames.com/")!)
        XCTAssertTrue(other.isEmpty, "Another site's page")
        let community = await fixture.overlay.webCookies(for: URL(string: "https://steamcommunity.com/app/620")!)
        XCTAssertEqual(community.count, 1)
        XCTAssertEqual(requests.accounts, [Self.account], "Kept a while")
    }

    // MARK: Fixture

    /// A signed-in account's overlay, with PEAK in front once it runs.
    @MainActor
    private func signedIn() async throws -> (OverlayFixture, SteamModel, SteamGameSessions) {
        let session = ChatBridgeTestSession(games: [Self.peak])
        let sessions = SteamGameSessions()
        let model = friendsTestModel(connect: { _ in session }, gameSessions: sessions)
        let fixture = OverlayFixture(steam: model)
        fixture.frontmost = Self.game
        await model.start()
        try await eventually { model.friends?.connection == .connected && !model.library.games.isEmpty }
        return (fixture, model, sessions)
    }

    private func peak(_ request: SteamOverlayRequest?, id: String = "peak", invites: [SteamReceivedInvite] = [],
                      microTxn: SteamMicroTxn? = nil, lobby: UInt64? = nil) -> GameSessionState {
        var state = GameSessionState.steam(appID: Self.peak.appId, accountID: Self.account, id: id)
        state.overlayRequest = request
        state.receivedInvites = invites
        state.microTxn = microTxn
        state.lobbyId = lobby
        return state
    }
}

private final class WebSignInRequests: @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [UInt64] = []
    var accounts: [UInt64] { lock.withLock { recorded } }
    func record(_ account: UInt64) { lock.withLock { recorded.append(account) } }
}
