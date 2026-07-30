pub use discord_sdk as ds;

/// A connected Discord RPC session.
pub struct Client {
    pub discord: ds::Discord,
    pub wheel: ds::wheel::Wheel,
    pub user: ds::user::User,
}

/// Opens a Discord RPC connection for `app_id`.
///
/// Every failure path returns an `Err` instead of panicking: Discord simply not
/// being running is an entirely normal situation and must not take the app down
/// with it.
pub async fn make_client(app_id: ds::AppId, subs: ds::Subscriptions) -> Result<Client, String> {
    println!("Creating Discord client with app ID: {}", app_id);

    let (wheel, handler) = ds::wheel::Wheel::new(Box::new(|err| {
        eprintln!("Discord error: {:?}", err);
    }));

    let mut user = wheel.user();

    let discord = ds::Discord::new(ds::DiscordApp::PlainId(app_id), subs, Box::new(handler))
        .map_err(|e| format!("Unable to create Discord client: {}", e))?;

    user.0
        .changed()
        .await
        .map_err(|e| format!("Discord closed the connection while connecting: {}", e))?;

    let user = match &*user.0.borrow() {
        ds::wheel::UserState::Connected(user) => user.clone(),
        ds::wheel::UserState::Disconnected(err) => {
            return Err(format!("Failed to connect to Discord: {}", err));
        }
    };

    println!("Connected to Discord, local user is {:#?}", user);

    Ok(Client {
        discord,
        wheel,
        user,
    })
}
