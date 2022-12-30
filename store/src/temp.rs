use std::{
    collections::{hash_map::Entry, HashMap},
    path::{Path, PathBuf},
    // time::SystemTime,
};

use base64::{
    alphabet::URL_SAFE,
    engine::fast_portable::{FastPortable, NO_PAD},
};
use rand::random;

struct Temporary {
    path: PathBuf,
}

/// Container for temporary files.
pub struct Temporaries<'a> {
    base: &'a Path,
    map: HashMap<u128, Temporary>,
}

impl<'a> Temporaries<'a> {
    pub fn new(base: &'a Path) -> Self {
        std::fs::create_dir_all(base).unwrap();
        Temporaries {
            base,
            map: HashMap::new(),
        }
    }

    pub fn make_temporary(&mut self) -> PathBuf {
        let tmp = loop {
            let key = random();
            match self.map.entry(key) {
                Entry::Occupied(_) => continue,
                Entry::Vacant(entry) => {
                    break entry.insert(Temporary {
                        path: {
                            let key_bytes = key.to_le_bytes();
                            let mut filename = [0; 22];
                            let len = base64::encode_engine_slice(
                                &key_bytes,
                                &mut filename,
                                &FastPortable::from(&URL_SAFE, NO_PAD),
                            );
                            debug_assert_eq!(len, 22);
                            self.base.join(std::str::from_utf8(&filename).unwrap())
                        },
                    });
                }
            }
        };
        tmp.path.clone()
    }

    pub fn clear(&mut self) {
        std::fs::remove_dir_all(self.base).unwrap();
    }
}

impl Drop for Temporaries<'_> {
    fn drop(&mut self) {
        self.clear();
    }
}
