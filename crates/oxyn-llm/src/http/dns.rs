//! Enforce the measured reach on every new HTTP connection (I-04).

use std::io;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::reach::Reach;

pub(super) struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            // Tokio runs the system resolver off the async workers (I-05).
            let addresses = tokio::net::lookup_host((name.as_str(), 0)).await?;
            Ok(Box::new(addresses.collect::<Vec<_>>().into_iter()) as Addrs)
        })
    }
}

pub(super) struct ReachResolver {
    pub(super) reach: Reach,
    pub(super) resolver: Arc<dyn Resolve>,
}

impl Resolve for ReachResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let pending = self.resolver.resolve(name);
        let reach = self.reach;
        Box::pin(async move {
            let addresses: Vec<_> = pending.await?.collect();
            // Check the whole answer before yielding any address: falling back to
            // a public answer after a loopback connection fails would leak context.
            if addresses.is_empty()
                || (reach == Reach::Local
                    && addresses.iter().any(|address| !address.ip().is_loopback()))
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "DNS answer violates the provider's measured reach",
                )
                .into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}
