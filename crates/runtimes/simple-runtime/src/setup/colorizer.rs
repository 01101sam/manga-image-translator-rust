use std::collections::HashMap;

use interface_colorizer::NoneColorizer;
use strum::IntoEnumIterator;

use crate::settings::Colorizer;

pub type ColorizerType = Box<dyn interface_colorizer::Colorizer + Send + Sync>;

pub struct Colorizers(HashMap<Colorizer, ColorizerType>);

impl Colorizers {
    pub fn get(&self, colorizer: Colorizer) -> &ColorizerType {
        self.0.get(&colorizer).expect("Colorizer not registered")
    }

    pub fn new() -> Self {
        let mut items = HashMap::new();
        for key in Colorizer::iter() {
            let colorizer = match key {
                Colorizer::None => Box::new(NoneColorizer) as ColorizerType,
            };
            items.insert(key, colorizer);
        }
        Colorizers(items)
    }
}
