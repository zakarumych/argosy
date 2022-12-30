// impl Source for Store {
//     type Error = eyre::Report;

//     fn find(&self, key: &str, asset: &str) -> BoxFuture<Option<AssetId>> {
//         self.find_asset(key, asset);

//         match self.base_url.join(key) {
//             Err(_) => {
//                 tracing::debug!("Key '{}' is not valid URL. It cannot be treasury key", key);
//                 Box::pin(async { None })
//             }
//             Ok(url) => {
//                 let treasury = self.treasury.clone();
//                 let asset: Box<str> = asset.into();
//                 Box::pin(async move {
//                     match treasury.lock().await.find(&url, &asset).await {
//                         Ok(None) => None,
//                         Ok(Some((tid, _))) => {
//                             let id = AssetId(tid.value());
//                             Some(id)
//                         }
//                         Err(err) => {
//                             tracing::error!("Failed to find '{}' in treasury. {:#}", url, err);
//                             None
//                         }
//                     }
//                 })
//             }
//         }
//     }

//     fn load(&self, id: AssetId) -> BoxFuture<Result<Option<AssetData>, Self::Error>> {
//         let tid = AssetId::from(id.0);

//         let treasury = self.treasury.clone();
//         Box::pin(async move {
//             match treasury.lock().await.fetch(tid).await {
//                 Ok(None) => Ok(None),
//                 Ok(Some(url)) => asset_data_from_url(url).await.map(Some),
//                 Err(err) => Err(TreasuryError::Treasury { source: err }),
//             }
//         })
//     }

//     fn update(
//         &self,
//         _id: AssetId,
//         _version: u64,
//     ) -> BoxFuture<Result<Option<AssetData>, Self::Error>> {
//         Box::pin(async { Ok(None) })
//     }
// }

// async fn asset_data_from_url(url: Url) -> Result<AssetData, TreasuryError> {
//     match url.scheme() {
//         "file" => match url.to_file_path() {
//             Err(()) => Err(TreasuryError::UrlError { url }),
//             Ok(path) => match tokio::runtime::Handle::try_current() {
//                 Err(_) => match std::fs::read(&path) {
//                     Ok(data) => Ok(AssetData {
//                         bytes: data.into_boxed_slice(),
//                         version: 0,
//                     }),
//                     Err(err) => Err(TreasuryError::IoError {
//                         source: err,
//                         path: path.into_boxed_path(),
//                     }),
//                 },
//                 Ok(runtime) => {
//                     let result = runtime
//                         .spawn_blocking(move || match std::fs::read(&path) {
//                             Ok(data) => Ok(AssetData {
//                                 bytes: data.into_boxed_slice(),
//                                 version: 0,
//                             }),
//                             Err(err) => Err(TreasuryError::IoError {
//                                 source: err,
//                                 path: path.into_boxed_path(),
//                             }),
//                         })
//                         .await;
//                     match result {
//                         Ok(Ok(data)) => Ok(data),
//                         Ok(Err(err)) => Err(err),
//                         Err(err) => Err(TreasuryError::JoinError { source: err }),
//                     }
//                 }
//             },
//         },
//         _ => Err(TreasuryError::UnsupportedUrlScheme { url }),
//     }
// }
