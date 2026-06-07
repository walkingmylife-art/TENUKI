use windows::core::PCWSTR;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW,
    PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_LARGE,
};

pub struct PdhQuery {
    query: isize,
    dedicated_counter: isize,
    shared_counter: Option<isize>,
}

impl PdhQuery {
    pub fn open() -> Option<Self> {
        unsafe {
            let mut query: isize = 0;
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }

            let dedicated_path: Vec<u16> = "\\GPU Adapter Memory(*)\\Dedicated Usage\0"
                .encode_utf16()
                .collect();
            let shared_path: Vec<u16> = "\\GPU Adapter Memory(*)\\Shared Usage\0"
                .encode_utf16()
                .collect();

            let mut dedicated_counter: isize = 0;
            if PdhAddEnglishCounterW(
                query,
                PCWSTR::from_raw(dedicated_path.as_ptr()),
                0,
                &mut dedicated_counter,
            ) != 0
            {
                PdhCloseQuery(query);
                return None;
            }

            let mut shared_counter: isize = 0;
            let shared_counter = if PdhAddEnglishCounterW(
                query,
                PCWSTR::from_raw(shared_path.as_ptr()),
                0,
                &mut shared_counter,
            ) == 0
            {
                Some(shared_counter)
            } else {
                None
            };

            let _ = PdhCollectQueryData(query);

            Some(Self {
                query,
                dedicated_counter,
                shared_counter,
            })
        }
    }

    fn collect_counter_mb(&self, counter: isize) -> Option<f32> {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }

            let mut buffer_size: u32 = 0;
            let mut item_count: u32 = 0;

            let _ = PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_LARGE,
                &mut buffer_size,
                &mut item_count,
                None,
            );

            if buffer_size == 0 || item_count == 0 {
                return None;
            }

            let mut buffer = vec![0u8; buffer_size as usize];
            if PdhGetFormattedCounterArrayW(
                counter,
                PDH_FMT_LARGE,
                &mut buffer_size,
                &mut item_count,
                Some(buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
            ) != 0
            {
                return None;
            }

            let items = std::slice::from_raw_parts(
                buffer.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W,
                item_count as usize,
            );

            let bytes = items
                .iter()
                .map(|item| item.FmtValue.Anonymous.largeValue)
                .max()
                .unwrap_or(0)
                .max(0) as f32;

            Some(bytes / (1024.0 * 1024.0))
        }
    }

    pub fn collect_dedicated_mb(&self) -> Option<f32> {
        self.collect_counter_mb(self.dedicated_counter)
    }

    pub fn collect_shared_mb(&self) -> Option<f32> {
        self.shared_counter
            .and_then(|counter| self.collect_counter_mb(counter))
    }
}

impl Drop for PdhQuery {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}
