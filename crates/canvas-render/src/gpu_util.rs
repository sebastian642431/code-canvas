/// A GPU buffer that grows (to the next power of two) when the data written
/// to it does not fit. Used for per-frame geometry.
pub struct DynBuffer {
    pub buf: wgpu::Buffer,
    cap: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl DynBuffer {
    pub fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let cap = 64 * 1024;
        Self { buf: Self::alloc(device, label, usage, cap), cap, usage, label }
    }

    fn alloc(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let len = data.len() as u64;
        if len > self.cap {
            self.cap = len.next_power_of_two();
            self.buf = Self::alloc(device, self.label, self.usage, self.cap);
        }
        // write_buffer needs a multiple of 4 bytes; our instances always are.
        queue.write_buffer(&self.buf, 0, data);
    }
}
