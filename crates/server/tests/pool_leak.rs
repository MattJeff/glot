//! A connection returned to the pool inside an open transaction is dropped, never handed to the next user.
use sqlx::Executor;

#[tokio::test]
async fn leaked_transaction_is_not_reused() {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        return;
    };
    let pool = glot_server::pg_pool_options()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    {
        let mut c = pool.acquire().await.unwrap();
        // What a cancelled sqlx BEGIN leaves behind: the server is in a transaction, sqlx doesn't know.
        c.execute("BEGIN READ ONLY").await.unwrap();
        sqlx::query("SELECT 1").execute(&mut *c).await.unwrap();
    }
    let mut c = pool.acquire().await.unwrap();
    let ro: String = sqlx::query_scalar("SELECT current_setting('transaction_read_only')")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(
        ro, "off",
        "the leaked read-only transaction was handed to the next user"
    );
}
