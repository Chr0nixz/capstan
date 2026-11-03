use super::*;
use capstan_core::sftp::ExactU64;

#[tokio::test]
async fn cwd_is_bound_to_window_terminal_view_and_authenticated_connection_generation() {
    let fixture = Fixture::new().await;
    let one = fixture.attach("one").await;
    let two = fixture.attach("two").await;
    let generation = ExactU64(fixture.lease.connection().connection_generation());
    let page = fixture.list(one, "/").await.unwrap();
    let stats = fixture.observations.stats.load(Ordering::SeqCst);
    let update = |window, terminal, session, generation, sequence, uri| {
        fixture.service.update_cwd(
            window,
            terminal,
            session,
            generation,
            ExactU64(sequence),
            uri,
        )
    };
    let first = update("main", "one", one, generation, 2, "file://127.0.0.1/one").unwrap();
    assert_eq!(first.path.unwrap().as_str(), "/one");
    for (window, terminal, session, generation) in [
        ("foreign", "one", one, generation),
        ("main", "two", one, generation),
        ("main", "one", two, generation),
        ("main", "one", one, ExactU64(generation.0 + 1)),
    ] {
        assert_eq!(
            update(
                window,
                terminal,
                session,
                generation,
                3,
                "file://127.0.0.1/wrong"
            ),
            Err(SftpError::ScopeExpired)
        );
    }
    assert!(fixture
        .service
        .configure_cwd_aliases("main", "two", two, generation, None)
        .unwrap()
        .path
        .is_none());
    let late = update("main", "one", one, generation, 1, "file://127.0.0.1/late").unwrap();
    assert_eq!(late.path.unwrap().as_str(), "/one");
    let wrong_host = update(
        "main",
        "one",
        one,
        generation,
        3,
        "file://localhost/not-implicit",
    )
    .unwrap();
    assert_eq!(wrong_host.path.unwrap().as_str(), "/one");
    // Hint validation must not issue SFTP reads, replace entry capabilities or mutate files.
    assert_eq!(fixture.observations.stats.load(Ordering::SeqCst), stats);
    fixture
        .service
        .prepare_mutation("main", "one", one, page.entries[0].id, Mutation::Delete)
        .await
        .unwrap();
    fixture.service.detach("main", "one", one).await.unwrap();
    assert_eq!(
        update(
            "main",
            "one",
            one,
            generation,
            4,
            "file://127.0.0.1/old-view"
        ),
        Err(SftpError::ScopeExpired)
    );
    let replacement = fixture.attach("one").await;
    assert_eq!(
        fixture
            .service
            .configure_cwd_aliases("main", "one", replacement, generation, None)
            .unwrap()
            .path
            .unwrap()
            .as_str(),
        "/one"
    );
    fixture.service.revoke_terminal("one");
    assert_eq!(
        update(
            "main",
            "one",
            replacement,
            generation,
            5,
            "file://127.0.0.1/revoked"
        ),
        Err(SftpError::ScopeExpired)
    );
    fixture.close().await;
}

#[tokio::test]
async fn explicit_alias_is_terminal_scoped_and_navigation_still_uses_sftp_authorization() {
    let fixture = Fixture::new().await;
    let one = fixture.attach("one").await;
    let two = fixture.attach("two").await;
    let generation = ExactU64(fixture.lease.connection().connection_generation());
    fixture
        .service
        .update_cwd(
            "main",
            "one",
            one,
            generation,
            ExactU64(1),
            "file://shell-host/empty",
        )
        .unwrap();
    let bound = fixture
        .service
        .configure_cwd_aliases(
            "main",
            "one",
            one,
            generation,
            Some(vec!["shell-host".into()]),
        )
        .unwrap();
    let path = bound.path.unwrap();
    let page = fixture
        .service
        .list("main", "one", one, path)
        .await
        .unwrap();
    assert_eq!(page.path.as_str(), "/empty");
    assert!(fixture
        .service
        .update_cwd(
            "main",
            "two",
            two,
            generation,
            ExactU64(1),
            "file://shell-host/empty"
        )
        .unwrap()
        .path
        .is_none());
    assert!(fixture
        .service
        .configure_cwd_aliases("main", "one", one, generation, Some(vec![]))
        .unwrap()
        .path
        .is_none());
    fixture.close().await;
}
