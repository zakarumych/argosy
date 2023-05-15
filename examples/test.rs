use std::convert::Infallible;

use argosy::{Asset, AssetData, AssetDriver, AssetField, Loader, Source};
use argosy_id::AssetId;
use futures::future::BoxFuture;

#[derive(Clone, Debug, Asset)]
pub struct Foo;

#[derive(Clone, Debug, AssetField)]
pub struct Bar {
    #[asset(external)]
    foo: Foo,
}

#[derive(Clone, Debug, Asset)]
pub struct WithFoo {
    #[asset(external)]
    foo: Foo,

    bar: Bar,
}

struct TestSource;

impl Source for TestSource {
    type Error = Infallible;

    fn find<'a>(&'a self, path: &'a str, asset: &'a str) -> BoxFuture<'a, Option<AssetId>> {
        match (path, asset) {
            ("WithFoo", "WithFoo") => Box::pin(async { Some(AssetId::new(2).unwrap()) }),
            _ => Box::pin(async { None }),
        }
    }

    fn load<'a>(&'a self, id: AssetId) -> BoxFuture<'a, Result<Option<AssetData>, Infallible>> {
        match id {
            AssetId(id) if id.get() == 1 => Box::pin(async {
                Ok(Some(AssetData {
                    bytes: (*b"{}").into(),
                    version: 0,
                }))
            }),
            AssetId(id) if id.get() == 2 => Box::pin(async {
                Ok(Some(AssetData {
                    bytes: (*b"{ \"foo\": 1, \"bar\": { \"foo\": 1 } }").into(),
                    version: 0,
                }))
            }),
            _ => Box::pin(async { Ok(None) }),
        }
    }

    fn update<'a>(
        &'a self,
        id: AssetId,
        _version: u64,
    ) -> BoxFuture<'a, Result<Option<AssetData>, Self::Error>> {
        self.load(id)
    }
}

fn main() {
    let loader = Loader::builder().with(TestSource).build();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let with_foo = loader.load::<WithFoo, _>("WithFoo");

        let with_foo_driver: AssetDriver<()> = with_foo.clone().driver();

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            with_foo_driver.await.build(&mut ());
        });

        let with_foo = with_foo.ready().await.unwrap();
        println!("{with_foo:?}");

        let with_foo = loader.load::<WithFoo, _>("WithFoo").ready().await.unwrap();
        println!("{with_foo:?}");
    })
}
