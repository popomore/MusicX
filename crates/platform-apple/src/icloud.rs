//! iCloud Documents transport. Never guess a Mobile Documents filesystem path.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use block2::{RcBlock, StackBlock};
use musicx_core::{
    cloud_sync::{self, merge::Replica},
    protocol::{CloudSyncPhase as Phase, tr},
};
use objc2::{
    AllocAnyThread,
    rc::{Retained, autoreleasepool},
    runtime::{AnyObject, ProtocolObject},
};
use objc2_foundation::{
    NSArray, NSBundle, NSFileCoordinator, NSFileCoordinatorReadingOptions,
    NSFileCoordinatorWritingOptions, NSFileManager, NSFileVersion, NSKeyedArchiver, NSMetadataItem,
    NSMetadataItemURLKey, NSMetadataQuery, NSMetadataQueryDidFinishGatheringNotification,
    NSMetadataQueryDidUpdateNotification, NSMetadataQueryUbiquitousDataScope, NSNotification,
    NSNotificationCenter, NSObjectProtocol, NSPredicate, NSString, NSURL,
};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{Mutex, OnceLock, mpsc},
    time::Duration,
};

static REQUESTS: OnceLock<mpsc::SyncSender<()>> = OnceLock::new();
static DISCOVERED: Mutex<Option<(PathBuf, Vec<PathBuf>, bool)>> = Mutex::new(None);
thread_local! { static WATCH: RefCell<Option<Discovery>> = const { RefCell::new(None) }; }

struct Discovery {
    directory: PathBuf,
    query: Retained<NSMetadataQuery>,
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.query.stopQuery();
        for observer in &self.observers {
            unsafe {
                NSNotificationCenter::defaultCenter().removeObserver(observer.as_ref());
            }
        }
    }
}

fn stop_discovery() {
    dispatch2::DispatchQueue::main().exec_async(|| {
        WATCH.with_borrow_mut(|watch| {
            watch.take();
        });
        *DISCOVERED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    });
}

/// Queries must start on the main thread. Results contain remote placeholders
/// even when filesystem enumeration has not materialized them on this device.
fn discover(directory: &Path) -> (Vec<PathBuf>, bool) {
    let requested = directory.to_owned();
    dispatch2::DispatchQueue::main().exec_async(move || {
        if !cloud_sync::enabled() {
            return;
        }
        WATCH.with_borrow_mut(|watch| {
            if watch.as_ref().is_some_and(|w| w.directory == requested) {
                return;
            }
            watch.take();
            *DISCOVERED.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((requested.clone(), Vec::new(), false));
            let query = NSMetadataQuery::new();
            query.setPredicate(Some(&NSPredicate::predicateWithValue(true)));
            let scope: &AnyObject = unsafe { NSMetadataQueryUbiquitousDataScope };
            unsafe {
                query.setSearchScopes(&NSArray::from_slice(&[scope]));
            }
            let mut observers = Vec::new();
            for notification in unsafe {
                [
                    NSMetadataQueryDidFinishGatheringNotification,
                    NSMetadataQueryDidUpdateNotification,
                ]
            } {
                let directory = requested.clone();
                let callback = RcBlock::new(move |notification: NonNull<NSNotification>| {
                    let Some(query) = (unsafe { notification.as_ref() })
                        .object()
                        .and_then(|o| o.downcast::<NSMetadataQuery>().ok())
                    else {
                        return;
                    };
                    query.disableUpdates();
                    let paths = query
                        .results()
                        .iter()
                        .filter_map(|item| {
                            let item = item.downcast::<NSMetadataItem>().ok()?;
                            let url = item
                                .valueForAttribute(unsafe { NSMetadataItemURLKey })?
                                .downcast::<NSURL>()
                                .ok()?;
                            let path = path(&url).ok()?;
                            if !path
                                .file_name()?
                                .to_str()?
                                .strip_suffix(".json")
                                .is_some_and(cloud_sync::merge::valid_device)
                            {
                                return None;
                            }
                            (path.parent().and_then(|p| p.canonicalize().ok()).as_deref()
                                == Some(directory.as_path()))
                            .then_some(path)
                        })
                        .collect();
                    query.enableUpdates();
                    *DISCOVERED.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((directory.clone(), paths, true));
                    wake();
                });
                let observer = unsafe {
                    NSNotificationCenter::defaultCenter()
                        .addObserverForName_object_queue_usingBlock(
                            Some(notification),
                            Some(&query),
                            None,
                            &callback,
                        )
                };
                observers.push(observer);
            }
            if !query.startQuery() {
                cloud_sync::set_phase(
                    Phase::Error,
                    Some("Could not discover iCloud sync files".into()),
                );
            }
            *watch = Some(Discovery {
                directory: requested,
                query,
                observers,
            });
        });
    });
    DISCOVERED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|(path, _, _)| path == directory)
        .map(|(_, paths, ready)| (paths.clone(), *ready))
        .unwrap_or_default()
}

pub fn start() -> Result<()> {
    if REQUESTS.get().is_some() {
        return Ok(());
    }
    let (send, receive) = mpsc::sync_channel(1);
    REQUESTS
        .set(send)
        .map_err(|_| anyhow::anyhow!("iCloud worker already started"))?;
    cloud_sync::set_waker(wake);
    std::thread::Builder::new()
        .name("musicx-icloud".into())
        .spawn(move || {
            loop {
                if cloud_sync::enabled() {
                    autoreleasepool(|_| {
                        if let Err(error) = exchange() {
                            cloud_sync::set_phase(Phase::Error, Some(error.to_string()));
                        }
                    });
                } else {
                    stop_discovery();
                }
                let _ = receive.recv_timeout(Duration::from_secs(30));
            }
        })?;
    Ok(())
}

fn wake() {
    if let Some(send) = REQUESTS.get() {
        let _ = send.try_send(());
    }
}

fn url(path: &Path) -> Result<objc2::rc::Retained<NSURL>> {
    Ok(NSURL::fileURLWithPath(&NSString::from_str(
        path.to_str().context("Invalid iCloud path")?,
    )))
}

fn path(url: &NSURL) -> Result<PathBuf> {
    Ok(PathBuf::from(
        url.path()
            .context("iCloud returned a non-file URL")?
            .to_string(),
    ))
}

fn coordinated<T>(url: &NSURL, writing: bool, action: impl Fn(&Path) -> Result<T>) -> Result<T> {
    let coordinator = NSFileCoordinator::initWithFilePresenter(NSFileCoordinator::alloc(), None);
    let output = RefCell::new(None);
    let accessor = StackBlock::new(|url: NonNull<NSURL>| {
        // Foundation supplies a valid URL for the duration of this synchronous callback.
        let result = path(unsafe { url.as_ref() }).and_then(|path| action(&path));
        *output.borrow_mut() = Some(result);
    });
    let mut error = None;
    if writing {
        coordinator.coordinateWritingItemAtURL_options_error_byAccessor(
            url,
            NSFileCoordinatorWritingOptions::ForReplacing,
            Some(&mut error),
            &accessor,
        );
    } else {
        coordinator.coordinateReadingItemAtURL_options_error_byAccessor(
            url,
            NSFileCoordinatorReadingOptions::empty(),
            Some(&mut error),
            &accessor,
        );
    }
    if let Some(error) = error {
        anyhow::bail!("{error}");
    }
    output
        .into_inner()
        .context("iCloud file coordination did not complete")?
}

fn read_replica(url: &NSURL) -> Result<Replica> {
    coordinated(url, false, |path| {
        cloud_sync::read_json(path)?.context("iCloud sync file is not downloaded yet")
    })
}

fn exchange() -> Result<()> {
    let container = NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&NSString::from_str("MusicXiCloudContainer"))
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|value| value.to_string());
    let Some(container) = container else {
        cloud_sync::set_phase(
            Phase::Unavailable,
            Some(
                tr!(
                    "This build does not include iCloud support. Install a build signed for iCloud."
                )
                .into(),
            ),
        );
        return Ok(());
    };
    cloud_sync::set_phase(Phase::Syncing, None);
    let manager = NSFileManager::defaultManager();
    let Some(token) = manager.ubiquityIdentityToken() else {
        stop_discovery();
        cloud_sync::set_phase(Phase::Unavailable, None);
        return Ok(());
    };
    // Apple's token contract guarantees NSCoding, not NSSecureCoding. This
    // archive is only compared as opaque bytes and is never unarchived.
    let archived = unsafe {
        NSKeyedArchiver::archivedDataWithRootObject_requiringSecureCoding_error(&token, false)
    }
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let account = STANDARD.encode(archived.to_vec());
    if !cloud_sync::bind_account(&account)? {
        stop_discovery();
        return Ok(());
    }
    let Some(root) =
        manager.URLForUbiquityContainerIdentifier(Some(&NSString::from_str(&container)))
    else {
        cloud_sync::set_phase(Phase::Unavailable, None);
        return Ok(());
    };
    let directory = path(&root)?.join("MusicXSync/v1");
    fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    let (discovered, ready) = discover(&directory);
    let mut files: BTreeSet<_> = discovered.into_iter().collect();
    let mut incoming = Replica::new(cloud_sync::device_id()?);
    let mut waiting = !ready;
    let mut conflicts = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(name) = name
            .strip_prefix('.')
            .and_then(|s| s.strip_suffix(".icloud"))
        {
            if name
                .strip_suffix(".json")
                .is_some_and(cloud_sync::merge::valid_device)
            {
                files.insert(directory.join(name));
            }
            continue;
        }
        if name
            .strip_suffix(".json")
            .is_some_and(cloud_sync::merge::valid_device)
        {
            ensure!(entry.file_type()?.is_file(), "Invalid iCloud sync file");
            files.insert(entry.path());
        }
    }
    ensure!(files.len() <= 256, "Too many iCloud device replicas");
    for file in files {
        let name = file
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid iCloud filename")?;
        let Some(device) = name
            .strip_suffix(".json")
            .filter(|s| cloud_sync::merge::valid_device(s))
        else {
            continue;
        };
        let file_url = url(&file)?;
        if !file.exists() {
            manager
                .startDownloadingUbiquitousItemAtURL_error(&file_url)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            waiting = true;
            continue;
        }
        ensure!(
            file.symlink_metadata()?.is_file(),
            "Invalid iCloud sync file"
        );
        let replica = read_replica(&file_url)?;
        ensure!(
            replica.device == device,
            "iCloud replica device does not match its filename"
        );
        incoming.merge(&replica)?;
        if let Some(versions) = NSFileVersion::unresolvedConflictVersionsOfItemAtURL(&file_url) {
            for version in versions {
                let replica = read_replica(&version.URL())?;
                ensure!(replica.device == device, "Invalid iCloud conflict version");
                incoming.merge(&replica)?;
                conflicts.push(version);
            }
        }
    }
    // Recheck the account immediately before local application and publication.
    if manager
        .ubiquityIdentityToken()
        .as_ref()
        .is_none_or(|current| {
            // Identity tokens implement NSObject equality, not pointer identity.
            let same: bool = unsafe { objc2::msg_send![&**current, isEqual: &*token] };
            !same
        })
    {
        cloud_sync::set_phase(Phase::AccountChanged, None);
        return Ok(());
    }
    let Some(replica) = cloud_sync::synchronize(&[incoming])? else {
        return Ok(());
    };
    crate::player::reload_preferences();
    if !cloud_sync::enabled() {
        return Ok(());
    }
    if manager
        .ubiquityIdentityToken()
        .as_ref()
        .is_none_or(|current| {
            let same: bool = unsafe { objc2::msg_send![&**current, isEqual: &*token] };
            !same
        })
    {
        cloud_sync::set_phase(Phase::AccountChanged, None);
        stop_discovery();
        return Ok(());
    }
    let output = url(&directory.join(format!("{}.json", replica.device)))?;
    let bytes = serde_json::to_vec(&replica)?;
    ensure!(
        bytes.len() as u64 <= cloud_sync::MAX_BYTES,
        "iCloud sync replica is too large"
    );
    coordinated(&output, true, |path| {
        if fs::read(path).ok().as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }
        let temp = path.with_extension(format!("{}.tmp", musicx_core::storage::random_hex::<8>()));
        let result = (|| -> Result<()> {
            fs::write(&temp, &bytes)?;
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    })?;
    // Mark versions resolved only after their merged contents are durable.
    for version in conflicts {
        version.setResolved(true);
    }
    cloud_sync::exchanged(waiting);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_coordination_reads_writes_and_propagates_accessor_errors() {
        autoreleasepool(|_| {
            let directory = std::env::temp_dir().join(format!(
                "musicx-file-coordination-{}",
                musicx_core::storage::random_hex::<12>()
            ));
            fs::create_dir_all(&directory).unwrap();
            let target = directory.join("replica.json");
            let target_url = url(&target).unwrap();
            let replica = Replica::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into());
            let bytes = serde_json::to_vec(&replica).unwrap();
            coordinated(&target_url, true, |path| {
                fs::write(path, &bytes)?;
                Ok(())
            })
            .unwrap();
            assert_eq!(read_replica(&target_url).unwrap(), replica);
            assert!(
                coordinated::<()>(&target_url, false, |_| anyhow::bail!("accessor failure"))
                    .unwrap_err()
                    .to_string()
                    .contains("accessor failure")
            );
            fs::remove_dir_all(directory).unwrap();
        });
    }
}
