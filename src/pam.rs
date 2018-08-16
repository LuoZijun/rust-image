#![feature(try_from, const_fn, duration_as_u128, nll)]
#![allow(unused_variables, unused_imports, unused_mut)]


use std::io;
use std::fmt;
use std::mem;
use std::cmp;
use std::thread;
use std::str::FromStr;
use std::convert::TryFrom;
use std::fs::{ File, OpenOptions };
use std::time::{ Duration, Instant };
use std::io::{ Read, Write, Seek, SeekFrom };


// http://netpbm.sourceforge.net/doc/pam.html

pub const SIGNATURE: [u8; 2] = [80, 55]; // b"P7"


#[derive(Debug)]
pub enum Error {
    IoError(io::Error),
    InvalidSignature,
    InvalidHeader,
    InvalidImageData,
    Other(&'static str),
}

impl From<io::Error> for Error {
    fn from(ioerr: io::Error) -> Error {
        Error::IoError(ioerr)
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    BlackAndWhite,
    Grayscale,
    RGB,
    BlackAndWhiteAlpha,
    GrayscaleAlpha,
    RGBA,
}

impl Color {
    pub fn channels(&self) -> u8 {
        match *self {
            Color::BlackAndWhite => 1,
            Color::Grayscale => 1,
            Color::RGB => 3,
            Color::BlackAndWhiteAlpha => 2,
            Color::GrayscaleAlpha => 2,
            Color::RGBA => 4,
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Color::BlackAndWhite => write!(f, "BLACKANDWHITE"),
            Color::Grayscale => write!(f, "GRAYSCALE"),
            Color::RGB => write!(f, "RGB"),
            Color::BlackAndWhiteAlpha => write!(f, "BLACKANDWHITE_ALPHA"),
            Color::GrayscaleAlpha => write!(f, "GRAYSCALE_ALPHA"),
            Color::RGBA => write!(f, "RGB_ALPHA"),
        }
    }
}

impl FromStr for Color {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "BLACKANDWHITE" => Ok(Color::BlackAndWhite),
            "GRAYSCALE" => Ok(Color::Grayscale),
            "RGB" => Ok(Color::RGB),
            "BLACKANDWHITE_ALPHA" => Ok(Color::BlackAndWhiteAlpha),
            "GRAYSCALE_ALPHA" => Ok(Color::GrayscaleAlpha),
            "RGB_ALPHA" => Ok(Color::RGBA),
            _ => Err(())
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub width: u64,
    pub height: u64,
    pub depth: u8,
    pub maxval: u16,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Data {
    pub offset: u64,
    pub length: u64,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Signature,
    Header,
    Data,
}

pub struct Decoder<Handle: Read + Seek> {
    state: State,
    handle: Handle,
    pixels_size: u64,
}

impl<Handle: Read + Seek> Decoder<Handle> {

    pub fn new(handle: Handle) -> Self {
        Decoder {
            state: State::Pending,
            handle: handle,
            pixels_size: 0,
        }
    }

    fn is_whitespace(&self, byte: u8) -> bool {
        // https://en.wikipedia.org/wiki/Whitespace_character
        // https://doc.rust-lang.org/beta/reference/whitespace.html
        // https://internals.rust-lang.org/t/should-bufread-lines-et-al-recognize-more-than-just-lf/1735/11
        // 
        // Tab  : 0x9   9 \t
        // LF   : 0xa  10 \n
        // CR   : 0xd  13 \r
        // Blank: 0x20 32
        // 
        // U+0009 (horizontal tab, '\t')
        // U+000A (line feed, '\n')
        // U+000B (vertical tab)
        // U+000C (form feed)
        // U+000D (carriage return, '\r')
        // U+0020 (space, ' ')
        // U+0085 (next line)
        // U+200E (left-to-right mark)
        // U+200F (right-to-left mark)
        // U+2028 (line separator)
        // U+2029 (paragraph separator)
        match byte {
            b'\t' | b'\n' | b' ' | 11 | b'\r' => true,
            _ => false,
        }
    }

    fn is_newline(&self, byte: u8) -> bool {
        // https://en.wikipedia.org/wiki/Newline
        match byte {
            b'\n' | b'\r' => true,
            _ => false,
        }
    }

    fn read_line_terminator(&mut self) -> Option<[u8; 2]> {
        let mut buffer = [0u8; 2];

        match self.handle.read(&mut buffer[..1]) {
            Ok(amt) => {
                if amt == 0 {
                    return None;
                }

                assert_eq!(amt, 1);
                let code = buffer[0];

                if !self.is_whitespace(code) {
                    return None;
                } 

                if code != b'\r'  {
                    return Some(buffer);
                }

                // FIXME: Support `\r\n` and `\n\r` ?
                if let Ok(amt) = self.handle.read(&mut buffer[1..]) {
                    if amt == 0 {
                        return None;
                    }
                    assert_eq!(amt, 1);

                    let code = buffer[1];
                    if code != b'\n'  {
                        // back
                        let pos = self.handle.seek(SeekFrom::Current(0)).unwrap();
                        self.handle.seek(SeekFrom::Start(pos - 1)).unwrap();
                        buffer[1] = 0u8;
                    }

                    return Some(buffer);
                } else {
                    return Some(buffer);
                }
            },
            Err(_) => None,
        }
    }

    pub fn read_signature(&mut self) -> Result<[u8; 2], Error> {
        let mut buffer = [0u8; 2];
        
        self.handle.seek(SeekFrom::Start(0)).unwrap();

        match self.handle.read_exact(&mut buffer) {
            Ok(_) => {
                self.state = State::Signature;
                let signature = buffer;

                assert_eq!(self.read_line_terminator().is_some(), true);
                // let line_terminator = self.read_line_terminator();
                // println!("Line terminator: {:?}", line_terminator);
                // assert_eq!(line_terminator.is_some(), true);

                Ok(signature)
            },
            Err(io_error) => Err(io_error.into())
        }
    }

    pub fn read_header(&mut self) -> Result<Header, Error> {
        assert_eq!(self.state, State::Signature);

        let mut buffer: [u8; 8] = [0u8; 8];
        
        let mut width: Option<u64> = None;
        let mut height: Option<u64> = None;
        // number of planes or channels
        let mut depth: Option<u8> = None;
        let mut maxval: Option<u16> = None;
        // WARN: As of 2015 PAM is not widely accepted or produced by graphics systems;
        //       e.g., XnView and FFmpeg support it.
        //       As specified the TUPLTYPE is optional;
        //       however, FFmpeg requires it.
        let mut tupltype: Option<Color> = None;

        loop {
            assert_eq!(self.handle.read(&mut buffer[..1]).unwrap(), 1);
            let first_char = buffer[0];
            let keysize = match first_char {
                b'W' => 5,
                b'H' => 6,
                b'D' => 5,
                b'M' => 6,
                b'T' => 8,
                b'E' => 6,
                b'#' => {
                    // Comment Line
                    loop {
                        assert_eq!(self.handle.read(&mut buffer[1..2]).unwrap(), 1);
                        
                        if self.is_newline(buffer[1]) {
                            let pos = self.handle.seek(SeekFrom::Current(0)).unwrap();
                            self.handle.seek(SeekFrom::Start(pos - 1)).unwrap();
                            break;
                        }
                    }
                    assert_eq!(self.read_line_terminator().is_some(), true);
                    continue;
                },
                c @ _ => {
                    println!("Unknow First Char: {:?}", first_char as char);
                    return Err(Error::InvalidHeader)
                },
            };
            self.handle.read(&mut buffer[1..keysize]).unwrap();

            assert_eq!(self.read_line_terminator().is_some(), true);

            if first_char == b'E' {
                assert_eq!(&buffer[..keysize], b"ENDHDR");
                break;
            }

            let mut val_buffer: [u8; 1] = [0u8; 1];
            let mut value: Vec<u8> = Vec::new();
            let mut value_index = 0usize;
            loop {
                assert_eq!(self.handle.read(&mut val_buffer).unwrap(), 1);
                
                if self.is_whitespace(val_buffer[0]) {
                    let pos = self.handle.seek(SeekFrom::Current(0)).unwrap();
                    self.handle.seek(SeekFrom::Start(pos - 1)).unwrap();
                    break;
                } else {
                    value.push(val_buffer[0]);
                }
            }

            let val_str = String::from_utf8(value).unwrap();

            match first_char {
                b'W' => {
                    assert_eq!(&buffer[..keysize], b"WIDTH");
                    assert_eq!(width.is_none(), true);
                    width = Some(val_str.parse::<u64>().unwrap());
                },
                b'H' => {
                    assert_eq!(&buffer[..keysize], b"HEIGHT");
                    assert_eq!(height.is_none(), true);
                    height = Some(val_str.parse::<u64>().unwrap());
                },
                b'D' => {
                    assert_eq!(&buffer[..keysize], b"DEPTH");
                    assert_eq!(depth.is_none(), true);
                    depth = Some(val_str.parse::<u8>().unwrap());
                },
                b'M' => {
                    assert_eq!(&buffer[..keysize], b"MAXVAL");
                    assert_eq!(maxval.is_none(), true);
                    maxval = Some(val_str.parse::<u16>().unwrap());
                },
                b'T' => {
                    assert_eq!(&buffer[..keysize], b"TUPLTYPE");
                    assert_eq!(tupltype.is_none(), true);
                    tupltype = Some(val_str.parse::<Color>().unwrap());
                },
                b'E' => {
                    unreachable!();
                },
                _ => unreachable!(),
            }

            assert_eq!(self.read_line_terminator().is_some(), true);
        }

        if width.is_none() || height.is_none() || depth.is_none() 
            || maxval.is_none() || tupltype.is_none() {
            return Err(Error::InvalidHeader);
        }

        let header = Header {
            width: width.unwrap(),
            height: height.unwrap(),
            depth: depth.unwrap(),
            maxval: maxval.unwrap(),
            color: tupltype.unwrap(),
        };

        // bytes per pixel
        let bpp = header.depth * (if header.maxval > 255 { 2 } else { 1 });
        let pixels_size = header.width * header.height * (bpp as u64);

        self.pixels_size = pixels_size;

        self.state = State::Header;

        Ok(header)
    }

    pub fn read_data(&mut self) -> Result<Data, Error> {
        assert_eq!(self.state, State::Header);
        assert_eq!(self.pixels_size > 0, true);

        let pos = self.handle.seek(SeekFrom::Current(0)).unwrap();

        self.state = State::Data;

        Ok(Data {
            offset: pos,
            length: self.pixels_size,
        })
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Signature([u8; 2]),
    Header(Header),
    Data(Data),
}

impl Element {
    
    pub fn is_signature(&self) -> bool {
        match *self {
            Element::Signature(_) => true,
            _ => false,
        }
    }

    pub fn is_header(&self) -> bool {
        match *self {
            Element::Header(_) => true,
            _ => false,
        }
    }

    pub fn is_data(&self) -> bool {
        match *self {
            Element::Data(_) => true,
            _ => false,
        }
    }

    pub fn signature(&self) -> [u8; 2] {
        match *self {
            Element::Signature(signature) => signature,
            _ => unreachable!(),
        }
    }

    pub fn header(&self) -> Header {
        match *self {
            Element::Header(header) => header,
            _ => unreachable!(),
        }
    }

    pub fn data(&self) -> Data {
        match *self {
            Element::Data(data) => data,
            _ => unreachable!(),
        }
    }
}

impl<Handle: Read + Seek> Iterator for Decoder<Handle> {
    type Item = Element;

    fn next(&mut self) -> Option<Self::Item> {
        if self.state == State::Pending {
            if let Ok(signature) = self.read_signature() {
                Some(Element::Signature(signature))
            } else {
                None
            }
        } else if self.state == State::Signature {
            if let Ok(header) = self.read_header() {
                Some(Element::Header(header))
            } else {
                None
            }
        } else if self.state == State::Header {
            if let Ok(data) = self.read_data() {
                Some(Element::Data(data))
            } else {
                None
            }
        } else {
            None
        }
    }
}


fn main (){
    let filepath = "output.pam";
    let mut file = File::open(filepath).unwrap();
    let mut decoder = Decoder::new(file.try_clone().unwrap());

    for elem in decoder {
        match elem {
            Element::Signature(signature) => {
                println!("Signature: {:?}", signature);
                assert_eq!(signature, SIGNATURE);
            },
            Element::Header(header) => {
                println!("{:?}", header);
            },
            Element::Data(data) => {
                println!("{:?}", data);

                // let mut pixels: Vec<u8> = vec![0u8; data.length as usize];
                // file.seek(SeekFrom::Start(data.offset)).unwrap();
                // assert_eq!(file.read(&mut pixels).unwrap(), data.length as usize);
                // println!("{:?}", pixels);
                
                println!("Pixel len: {:?} Bytes", data.length);
            },
        }
    }
}

