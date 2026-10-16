//! Locates secrets inside screenshots: OCR with the macOS Vision framework, then the same
//! detector used for copied text, mapped back to bounding boxes in the image.

use crate::secrets;

/// A secret's location in the image: normalized `[x, y, w, h]`, origin top-left.
pub struct Hit {
    pub rect: [f64; 4],
    pub label: &'static str,
}

#[cfg(target_os = "macos")]
pub fn find_secrets(png: &[u8]) -> Vec<Hit> {
    use objc2::{AnyThread, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSError, NSRange};
    use objc2_vision::{
        VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRectangleObservation,
        VNRequest, VNRequestTextRecognitionLevel,
    };

    let data = NSData::with_bytes(png);
    let options: Retained<NSDictionary<VNImageOption, AnyObject>> = NSDictionary::new();
    let handler = VNImageRequestHandler::initWithData_options(
        VNImageRequestHandler::alloc(),
        &data,
        &options,
    );

    let request = VNRecognizeTextRequest::new();
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(false); // keys aren't words; "correcting" them hurts

    let as_request: &VNRequest = &request;
    if handler
        .performRequests_error(&NSArray::from_slice(&[as_request]))
        .is_err()
    {
        return Vec::new();
    }
    let Some(results) = request.results() else {
        return Vec::new();
    };

    let mut hits = Vec::new();
    for i in 0..results.count() {
        let observation = results.objectAtIndex(i);
        let Some(line) = observation.topCandidates(1).firstObject() else {
            continue;
        };
        // OCR often returns look-alike letters and fancy dashes; clean them up (1:1, so offsets hold).
        let text = secrets::normalize_ocr(&line.string().to_string());

        for found in secrets::scan_text(&text) {
            // Vision wants UTF-16 offsets.
            let start = text[..found.range.start].encode_utf16().count();
            let len = text[found.range.clone()].encode_utf16().count();
            let range = NSRange::new(start, len);

            // The box around the secret. Vision resolves ranges per word, so `KEY=value` (no spaces)
            // is boxed whole: that over-hides slightly but never under-hides. Falls back to the line.
            let precise: Result<Retained<VNRectangleObservation>, Retained<NSError>> =
                unsafe { msg_send![&*line, boundingBoxForRange: range, error: _] };
            // SAFETY: both are live observation objects; `boundingBox` is a plain getter.
            let bb = unsafe {
                match &precise {
                    Ok(o) => o.boundingBox(),
                    Err(_) => observation.boundingBox(),
                }
            };
            // Vision's origin is bottom-left; flip to top-left.
            hits.push(Hit {
                rect: [
                    bb.origin.x,
                    1.0 - bb.origin.y - bb.size.height,
                    bb.size.width,
                    bb.size.height,
                ],
                label: found.label,
            });
        }
    }
    hits
}

#[cfg(not(target_os = "macos"))]
pub fn find_secrets(_png: &[u8]) -> Vec<Hit> {
    Vec::new()
}
