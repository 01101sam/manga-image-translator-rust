use std::{collections::HashMap, sync::Arc};

use base_util::onnx::all_providers;
use strum::IntoEnumIterator;

use crate::settings::Detector;
pub type DetectorType = Box<dyn interface_detector::Detector + Send + Sync>;

pub struct Detectors(HashMap<Detector, DetectorType>);
impl Detectors {
    pub fn get(&self, detector: Detector) -> &DetectorType {
        self.0.get(&detector).expect("Detector not registered")
    }
    pub fn new() -> Self {
        let mut items = HashMap::new();
        let providers = Arc::new(all_providers());
        for detector_key in Detector::iter() {
            let detector = match detector_key {
                Detector::DBNet => {
                    // allow:clone[arc]
                    Box::new(dbnet::DbNetDetector::new(providers.clone())) as DetectorType
                }
                // Detector::DBNetConvNext => todo!(),
                // allow:clone[arc]
                Detector::Ctd => Box::new(ctd::CtdDetector::new(providers.clone())) as DetectorType,
            };
            items.insert(detector_key, detector);
        }
        Detectors(items)
    }
}
