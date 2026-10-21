import SwiftUI

/// A game of any of the player's storefronts, as Home shows it beside the
/// other storefronts' games: what orders and names it, what its cover and hero
/// show, and what its buttons do.
@MainActor
struct HomeGame: Identifiable {
    /// The game within its storefront's library.
    struct ID: Hashable, Sendable {
        let section: LibrarySection
        let game: String
    }

    /// A storefront game's wide artwork and what names the game over it.
    struct Hero {
        let artwork: () -> AnyView
        let logo: () -> AnyView
    }

    let id: ID
    let title: String
    let lastPlayed: Date?
    let playtime: Duration?
    /// What the game's card and hero buttons show and do.
    let action: GameAction
    let cover: () -> AnyView
    let hero: Hero
    /// Opens the game's page in its storefront.
    let open: () -> Void
    /// What its cover's context menu offers to manage of its installation.
    var management: GameManagement?
}

/// One account's library as Home shows it, built only for the games Home lists.
@MainActor
struct HomeLibrary {
    let featured: HomeGame?
    /// Its played games, the last played first: one more than a shelf holds,
    /// as the hero may be one of them.
    let recent: [HomeGame]
    /// Its installed games, the last played first.
    let installed: [HomeGame]
    /// The first games of its last played order, as its tile shows them.
    let covers: [HomeGame]
    let count: Int
}

extension HomeGame {
    /// `game` of `library`, named, dated and timed as the library knows it.
    init<Store, Game>(_ game: Game, of library: AccountLibrary<Store, Game>, in section: LibrarySection,
                      action: GameAction, management: GameManagement? = nil, cover: @escaping () -> AnyView,
                      hero: Hero, open: @escaping () -> Void) {
        self.init(id: ID(section: section, game: "\(game.id)"), title: game.title,
                  lastPlayed: library.lastPlayed(of: game), playtime: library.playtime(of: game),
                  action: action, cover: cover, hero: hero, open: open, management: management)
    }
}

extension HomeLibrary {
    /// `library` as Home shows it; `home` makes each game Home lists.
    init<Store, Game>(_ library: AccountLibrary<Store, Game>, home: (Game) -> HomeGame) {
        let order = library.games(by: .lastPlayed)
        self.init(
            featured: library.featured.map(home),
            recent: order.prefix { library.lastPlayed(of: $0) != nil }.prefix(HomeShelves.shelfLimit + 1).map(home),
            installed: library.games(by: .lastPlayed, filter: .installed).map(home),
            covers: order.prefix(3).map(home),
            count: library.games.count
        )
    }
}

extension LibraryStorefront {
    /// The account's library as Home shows it, in the storefront's own artwork.
    func homeLibrary<Cover: View, Artwork: View, Logo: View>(
        in section: LibrarySection,
        cover: @escaping (Game) -> Cover,
        heroArtwork: @escaping (Game) -> Artwork,
        heroLogo: @escaping (Game) -> Logo
    ) -> HomeLibrary {
        let actions = GameActions(installations, in: .library)
        return HomeLibrary(library) { game in
            homeGame(game, in: section, actions: actions, cover: cover, heroArtwork: heroArtwork, heroLogo: heroLogo)
        }
    }

    /// The account's games whose title matches `query`, by name.
    func homeResults<Cover: View, Artwork: View, Logo: View>(
        matching query: String,
        in section: LibrarySection,
        cover: @escaping (Game) -> Cover,
        heroArtwork: @escaping (Game) -> Artwork,
        heroLogo: @escaping (Game) -> Logo
    ) -> [HomeGame] {
        let actions = GameActions(installations, in: .library)
        return library.games(by: .name, matching: query).map { game in
            homeGame(game, in: section, actions: actions, cover: cover, heroArtwork: heroArtwork, heroLogo: heroLogo)
        }
    }

    private func homeGame<Cover: View, Artwork: View, Logo: View>(
        _ game: Game,
        in section: LibrarySection,
        actions: GameActions<Store>,
        cover: @escaping (Game) -> Cover,
        heroArtwork: @escaping (Game) -> Artwork,
        heroLogo: @escaping (Game) -> Logo
    ) -> HomeGame {
        let selection = librarySelection(game)
        return HomeGame(
            game, of: library, in: section,
            action: actions(selection),
            management: installations.management(of: selection),
            cover: { AnyView(cover(game)) },
            hero: HomeGame.Hero(artwork: { AnyView(heroArtwork(game)) }, logo: { AnyView(heroLogo(game)) }),
            open: { self.open(game) }
        )
    }
}

/// What Home shows of every library at once.
@MainActor
struct HomeShelves {
    /// The game Home shows first: the last one played anywhere, else the most
    /// played, as a library's hero.
    let hero: HomeGame?
    /// The games played before the hero's, the last played first.
    let recent: [HomeGame]
    /// Every game on the Mac, the last played first.
    let installed: [HomeGame]

    static let shelfLimit = 20

    /// `libraries` in Home's order, which equal games across libraries keep.
    init(_ libraries: [HomeLibrary]) {
        let featured = Array(libraries.compactMap(\.featured).enumerated())
        let hero = LibraryOrder.featured(featured, title: { $0.element.title }, id: { $0.offset },
                                         lastPlayed: { $0.element.lastPlayed }, playtime: { $0.element.playtime })?.element
        self.hero = hero
        let recent = Self.lastPlayedFirst(libraries.map(\.recent))
        self.recent = Array(recent.filter { $0.id != hero?.id }.prefix(Self.shelfLimit))
        installed = Self.lastPlayedFirst(libraries.map(\.installed))
    }

    /// Merges each library's games, each in its last played order.
    private static func lastPlayedFirst(_ games: [[HomeGame]]) -> [HomeGame] {
        LibraryOrder.merge(games, title: \.title, rank: { $0.lastPlayed?.timeIntervalSince1970 })
    }
}
