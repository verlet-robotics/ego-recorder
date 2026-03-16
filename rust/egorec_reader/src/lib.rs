//! PyO3 bindings for egorec — exposes EgorecFile to Python for hand detection in intervals stage.
//!
//! API matches the C++ pybind11 egorec_reader:
//!   - EgorecFile(path)
//!   - header() -> dict with duration_s, frame_count, etc.
//!   - frame_count() -> int
//!   - frames() -> iterator yielding {"rgb": ndarray (H,W,3) uint8, ...}

use numpy::{ndarray::Array3, IntoPyArray};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use egorec::format::FileHeader;
use egorec::reader::{EgorecReader, FrameIterator as EgorecFrameIterator};

#[pyclass]
struct EgorecFile {
    frame_count: u64,
    duration_s: f64,
    header: FileHeader,
    reader: Option<EgorecReader>,
}

#[pymethods]
impl EgorecFile {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let reader = EgorecReader::open(path)
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e.to_string()))?;
        let frame_count = reader.frame_count();
        let duration_s = reader.duration_s();
        let header = reader.header().clone();
        Ok(Self {
            frame_count,
            duration_s,
            header,
            reader: Some(reader),
        })
    }

    fn frame_count(&self) -> u64 {
        self.frame_count
    }

    fn header(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let d = PyDict::new_bound(py);
        d.set_item("duration_s", self.duration_s)?;
        d.set_item("frame_count", self.frame_count)?;
        d.set_item("session_name", self.header.session_name_str())?;
        d.set_item("format_version", 2i32)?;
        d.set_item("start_ts_us", self.header.start_timestamp_us)?;
        d.set_item("depth_width", self.header.depth_width)?;
        d.set_item("depth_height", self.header.depth_height)?;
        d.set_item("color_width", self.header.color_width)?;
        d.set_item("color_height", self.header.color_height)?;
        d.set_item("serial_number", self.header.serial_number_str())?;
        d.set_item("usb_type", self.header.usb_type_str())?;
        d.set_item("has_imu", self.header.has_imu())?;
        Ok(d.into_py(py))
    }

    fn frames(mut slf: PyRefMut<'_, Self>) -> PyResult<EgorecFrameIteratorPy> {
        let reader = slf
            .reader
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("frames() can only be called once"))?;
        let frame_iter = reader
            .frames()
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e.to_string()))?;
        Ok(EgorecFrameIteratorPy {
            inner: frame_iter,
            color_width: slf.header.color_width as usize,
            color_height: slf.header.color_height as usize,
            depth_width: slf.header.depth_width as usize,
            depth_height: slf.header.depth_height as usize,
        })
    }
}

#[pyclass(unsendable)]
struct EgorecFrameIteratorPy {
    inner: EgorecFrameIterator,
    color_width: usize,
    color_height: usize,
    depth_width: usize,
    depth_height: usize,
}

#[pymethods]
impl EgorecFrameIteratorPy {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match slf.inner.next() {
            Some(Ok(frame)) => {
                let d = PyDict::new_bound(py);
                d.set_item("timestamp_us", frame.timestamp_us)?;
                d.set_item("timestamp_relative_s", frame.timestamp_relative_s)?;
                d.set_item("frame_number", frame.frame_number)?;
                let rgb_shape = [slf.color_height, slf.color_width, 3];
                let rgb_arr = Array3::from_shape_fn(rgb_shape, |(i, j, k)| {
                    let idx = (i * slf.color_width + j) * 3 + k;
                    frame.rgb.get(idx).copied().unwrap_or(0)
                });
                d.set_item("rgb", rgb_arr.into_pyarray_bound(py))?;
                let depth_shape = (slf.depth_height, slf.depth_width);
                let depth_arr = numpy::ndarray::Array2::from_shape_vec(
                    depth_shape,
                    frame.depth,
                ).map_err(|e| pyo3::exceptions::PyOSError::new_err(e.to_string()))?;
                d.set_item("depth", depth_arr.into_pyarray_bound(py))?;
                Ok(Some(d.into_py(py)))
            }
            Some(Err(e)) => Err(pyo3::exceptions::PyOSError::new_err(e.to_string())),
            None => Ok(None),
        }
    }
}

#[pymodule]
fn egorec_reader(m: &Bound<'_, pyo3::prelude::PyModule>) -> PyResult<()> {
    m.add_class::<EgorecFile>()?;
    Ok(())
}
