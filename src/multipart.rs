use mime::Mime;
use std::borrow::Cow;
use std::io::prelude::*;
use std::io::Cursor;
use std::io::Result;

pub struct Multipart<'d> {
	fields: Vec<(String, Data<'d>)>,
}

impl<'d> Multipart<'d> {
	pub fn new() -> Self {
		Self { fields: Vec::new() }
	}

	pub fn add_text(&mut self, name: impl ToString, text: impl Into<Cow<'d, str>>) {
		self.fields.push((name.to_string(), Data::Text(text.into())));
	}

	pub fn add_stream(
		&mut self,
		name: impl ToString,
		stream: impl Read + 'd,
		filename: Option<impl ToString>,
		mime: Option<Mime>,
	) {
		let data = Stream {
			content_type: mime.unwrap_or(mime::APPLICATION_OCTET_STREAM),
			filename: filename.map(|f| f.to_string()),
			stream: Box::new(stream),
		};
		self.fields.push((name.to_string(), Data::Stream(data)));
	}

	pub fn prepare(&mut self) -> Result<PreparedFields<'d>> {
		use rand::Rng;
		let mut boundary = format!(
			"\r\n--{}",
			rand::thread_rng()
				.sample_iter(&rand::distributions::Alphanumeric)
				.take(16)
				.map(|c| c as char)
				.collect::<String>()
		);

		let mut text_data = Vec::new();
		let mut streams = Vec::new();

		for field in self.fields.drain(..) {
			match field.1 {
				Data::Text(text) => write!(
					text_data,
					"{}\r\nContent-Disposition: form-data; \
                     name=\"{}\"\r\n\r\n{}",
					boundary, field.0, text
				)
				.unwrap(),
				Data::Stream(stream) => {
					streams.push(PreparedField::from_stream(
						&field.0,
						&boundary,
						&stream.content_type,
						stream.filename.as_ref().map(|f| &**f),
						stream.stream,
					));
				},
			}
		}

		if text_data.is_empty() && streams.is_empty() {
			boundary = String::new();
		} else {
			boundary.push_str("--");
		}

		Ok(PreparedFields {
			text_data: Cursor::new(text_data),
			streams,
			end_boundary: Cursor::new(boundary),
		})
	}
}

enum Data<'d> {
	Text(Cow<'d, str>),
	Stream(Stream<'d>),
}

struct Stream<'d> {
	filename: Option<String>,
	content_type: Mime,
	stream: Box<dyn Read + 'd>,
}

pub struct PreparedFields<'d> {
	text_data: Cursor<Vec<u8>>,
	streams: Vec<PreparedField<'d>>,
	end_boundary: Cursor<String>,
}

impl<'d> PreparedFields<'d> {
	pub fn boundary(&self) -> &str {
		let boundary = self.end_boundary.get_ref();

		&boundary[4..boundary.len() - 2]
	}
}

impl<'d> Read for PreparedFields<'d> {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
		if buf.is_empty() {
			return Ok(0);
		}

		let mut total_read = 0;

		while total_read < buf.len() && !cursor_at_end(&self.end_boundary) {
			let buf = &mut buf[total_read..];

			total_read += if !cursor_at_end(&self.text_data) {
				self.text_data.read(buf)?
			} else if let Some(mut field) = self.streams.pop() {
				match field.read(buf) {
					Ok(0) => continue,
					res => {
						self.streams.push(field);
						res
					},
				}?
			} else {
				self.end_boundary.read(buf)?
			};
		}

		Ok(total_read)
	}
}

struct PreparedField<'d> {
	header: Cursor<Vec<u8>>,
	stream: Box<dyn Read + 'd>,
}

impl<'d> PreparedField<'d> {
	fn from_stream(
		name: &str,
		boundary: &str,
		content_type: &Mime,
		filename: Option<&str>,
		stream: Box<dyn Read + 'd>,
	) -> Self {
		let mut header = Vec::new();

		write!(header, "{}\r\nContent-Disposition: form-data; name=\"{}\"", boundary, name)
			.unwrap();

		if let Some(filename) = filename {
			write!(header, "; filename=\"{}\"", filename).unwrap();
		}

		write!(header, "\r\nContent-Type: {}\r\n\r\n", content_type).unwrap();

		PreparedField { header: Cursor::new(header), stream }
	}
}

impl<'d> Read for PreparedField<'d> {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
		if !cursor_at_end(&self.header) {
			self.header.read(buf)
		} else {
			self.stream.read(buf)
		}
	}
}

fn cursor_at_end<T: AsRef<[u8]>>(cursor: &Cursor<T>) -> bool {
	cursor.position() == (cursor.get_ref().as_ref().len() as u64)
}
