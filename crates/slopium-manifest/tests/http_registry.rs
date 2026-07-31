//! The one part of a registry that is not just a directory: the transport.
//!
//! A registry is a static tree, so `file://` covers the whole of the index
//! format and everything that reads it — which is why `scripts/registry-check.sh`
//! uses a directory. What a directory cannot exercise is `curl`, the status
//! codes it has to tell apart, and the loopback rule that lets a test have a
//! server at all (`D-052`). That is this file.
//!
//! Nothing here reaches the network. The server is a `TcpListener` on
//! `127.0.0.1:0` serving a tree this test wrote, which is what makes it
//! runnable inside `nix flake check`'s sandbox.

use slopium_manifest::manifest::{LocalConfig, RegistryConfig};
use slopium_manifest::registry::{IndexDependency, IndexEntry, IndexSource, Registries, Registry};
use slopium_manifest::sha256::sha256;
use slopium_manifest::sources::Sources;
use slopium_manifest::store::{Access, Store};
use slopium_manifest::version::{Version, VersionReq};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;

/// Serve a directory over HTTP until the process ends, and answer with the
/// port it got.
fn serve(root: PathBuf) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback is available");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = answer(stream, &root);
        }
    });
    port
}

fn answer(mut stream: TcpStream, root: &Path) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    // The rest of the headers have to be drained, or the client sees the
    // connection close under a request it thinks it is still sending.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
    }

    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let relative = path.trim_start_matches('/');
    // A registry never asks for anything outside itself, and a server that
    // would answer if it did is not one to write even in a test.
    let response = match relative.contains("..") {
        true => None,
        false => fs::read(root.join(relative)).ok(),
    };
    match response {
        Some(body) => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(&body)?;
        }
        None => write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )?,
    }
    stream.flush()
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "slopium-http-{label}-{}-{:?}",
        std::process::id(),
        thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn an_index_and_an_archive_are_read_over_http() {
    let root = scratch("served");
    let archive = b"not really a tar, but the transport does not read it".to_vec();
    let entry = IndexEntry {
        name: "geometry".to_owned(),
        version: Version::new(1, 4, 0),
        dependencies: vec![IndexDependency {
            name: "std".to_owned(),
            requirement: VersionReq::parse("^0.4").unwrap(),
            source: IndexSource::Toolchain,
        }],
        checksum: sha256(&archive),
        yanked: false,
        signature: None,
    };
    let index = root.join("index/ge/om/geometry.json");
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    fs::write(&index, format!("{}\n", entry.render().unwrap())).unwrap();
    let package = root.join("packages/geometry/geometry-1.4.0.sl.tar");
    fs::create_dir_all(package.parent().unwrap()).unwrap();
    fs::write(&package, &archive).unwrap();

    let port = serve(root.clone());
    let registry = Registry::new(
        "test",
        &format!("http://127.0.0.1:{port}"),
        Path::new("/nonexistent"),
    )
    .unwrap();

    let published = registry.versions("geometry").unwrap();
    assert_eq!(published.as_slice(), std::slice::from_ref(&entry));
    assert_eq!(
        registry.archive("geometry", &entry.version).unwrap(),
        archive
    );

    // A package the registry does not have is not an error: it is the answer
    // to "which versions of this are published here", and it is none.
    assert!(registry.versions("absent").unwrap().is_empty());

    let _ = fs::remove_dir_all(&root);
}

/// The index cache, and the three rules that make it safe to have one:
/// `--offline` reads it, an online run never does, and an online run that finds
/// a package gone throws the stale copy away.
///
/// The proof that offline is really reading the cache is that the file it was
/// fetched from is deleted first — a run that reached the server would get a
/// 404 and answer "no versions", which is exactly what the online run below it
/// does.
#[test]
fn a_fetched_index_is_cached_and_then_read_offline() {
    let served = scratch("cached-registry");
    let home = scratch("cached-home");
    let entry = IndexEntry {
        name: "geometry".to_owned(),
        version: Version::new(1, 4, 0),
        dependencies: Vec::new(),
        checksum: sha256(b"geometry"),
        yanked: false,
        signature: None,
    };
    let index_file = served.join("index/ge/om/geometry.json");
    fs::create_dir_all(index_file.parent().unwrap()).unwrap();
    fs::write(&index_file, format!("{}\n", entry.render().unwrap())).unwrap();

    let port = serve(served.clone());
    let url = format!("http://127.0.0.1:{port}");
    let published = |access| {
        let mut config = LocalConfig::default();
        config.registry.insert(
            "test".to_owned(),
            RegistryConfig {
                index: Some(url.clone()),
                trusted_keys: Vec::new(),
            },
        );
        let registries = Registries::from_config(&config, Path::new("/nonexistent")).unwrap();
        Sources::new(Store::at(&home), access, false)
            .with_registries(registries)
            .published("geometry", &url)
    };

    // Online: fetched, and written through to the cache.
    assert_eq!(published(Access::Online).unwrap(), vec![entry.clone()]);
    let cache_root = home.join("index").join(sha256(url.as_bytes()).to_string());
    let cached = cache_root.join("ge/om/geometry.json");
    assert!(cached.is_file(), "the fetched index file is kept");
    assert_eq!(
        fs::read_to_string(cache_root.join("url")).unwrap().trim(),
        url,
        "the cache directory says which registry it is"
    );

    fs::remove_file(&index_file).unwrap();

    // Offline: answered from the cache, though the server no longer has it.
    assert_eq!(published(Access::Offline).unwrap(), vec![entry]);

    // Online: the index is the authority (`D-055`), so a package it stopped
    // serving stops resolving, and the stale copy goes.
    assert!(published(Access::Online).unwrap().is_empty());
    assert!(!cached.exists(), "a package the index dropped is not kept");

    // And now there is nothing to be offline with, which has to say so.
    let error = published(Access::Offline).unwrap_err();
    assert!(error.contains("SL1011"), "{error}");
    assert!(error.contains(&cached.display().to_string()), "{error}");

    let _ = fs::remove_dir_all(&served);
    let _ = fs::remove_dir_all(&home);
}

/// A registry that is a directory is local, so `--offline` has nothing to
/// forbid about reading it and nothing worth caching.
#[test]
fn a_directory_registry_resolves_offline_without_a_cache() {
    let served = scratch("directory-registry");
    let entry = IndexEntry {
        name: "geometry".to_owned(),
        version: Version::new(2, 0, 0),
        dependencies: Vec::new(),
        checksum: sha256(b"geometry"),
        yanked: false,
        signature: None,
    };
    let index_file = served.join("index/ge/om/geometry.json");
    fs::create_dir_all(index_file.parent().unwrap()).unwrap();
    fs::write(&index_file, format!("{}\n", entry.render().unwrap())).unwrap();

    let url = format!("file://{}", served.display());
    let mut config = LocalConfig::default();
    config.registry.insert(
        "test".to_owned(),
        RegistryConfig {
            index: Some(url.clone()),
            trusted_keys: Vec::new(),
        },
    );
    let registries = Registries::from_config(&config, Path::new("/nonexistent")).unwrap();
    let found = Sources::new(Store::at("/nonexistent"), Access::Offline, false)
        .with_registries(registries)
        .published("geometry", &url)
        .unwrap();
    assert_eq!(found, vec![entry]);

    let _ = fs::remove_dir_all(&served);
}

/// `D-052`: whoever answers a plaintext index chooses what a first resolution
/// pins. Loopback is the one hop with nothing in between.
#[test]
fn a_plaintext_index_is_refused_off_loopback() {
    let error = Registry::new("public", "http://example.invalid/index", Path::new("/tmp"))
        .expect_err("a plaintext index off loopback is refused");
    assert!(error.contains("SL1030"), "{error}");
    assert!(error.contains("loopback"), "{error}");
}
