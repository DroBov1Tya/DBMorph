use mongodb::{
    bson::{doc, Document},
    options::{ClientOptions, Credential},
    Client, Collection,
};
use std::error::Error;

pub async fn mongo_connect(
    connection_uri: &str,
    db_user: &str,
    db_pass: &str,
) -> Result<Client, Box<dyn Error>> {
    // Builds MongoDB connection string with credentials
    // Parses client options and sets authentication credentials
    // Creates and returns a MongoDB client instance
    let conn = format!("mongodb://{}:{}@{}", db_user, db_pass, connection_uri);
    println!("ConnString: {}", conn);
    let mut client_options = ClientOptions::parse(conn).await?;

    client_options.credential = Some(
        Credential::builder()
            .username(Some(db_user.to_string()))
            .password(Some(db_pass.to_string()))
            .build(),
    );

    let client = Client::with_options(client_options)?;
    Ok(client)
}
