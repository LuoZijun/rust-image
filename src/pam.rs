#![feature(try_from, const_fn, duration_as_u128, nll)]
#![allow(unused_variables, unused_imports, unused_mut)]


// http://netpbm.sourceforge.net/doc/pam.html


mod netpbm;

pub use self::netpbm::{ PAM_BINARY_MAGIC_NUMBER, Lines };

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

pub struct Decoder<RS: Read + Seek> {
    state: State,
    line_reader: Lines<RS>,
    pixels_size: u64,
}

impl<RS: Read + Seek> Decoder<RS> {

    pub fn new(handle: RS) -> Self {
        Decoder {
            state: State::Pending,
            line_reader: Lines { handle: handle },
            pixels_size: 0,
        }
    }

    pub fn read_signature(&mut self) -> Result<[u8; 2], Error> {
        assert_eq!(self.state, State::Pending);
        self.line_reader.handle.seek(SeekFrom::Start(0)).unwrap();

        if let Some(line) = self.line_reader.next() {
            if line.len() == 2 {
                self.state = State::Signature;
                return Ok([ line[0], line[1], ])
            }
        }

        Err(Error::InvalidSignature)
    }

    fn next_value(&mut self) -> Option<String> {
        if let Some(line) = self.line_reader.next() {
            if line.len() > 0 {
                if line[0] == b'#' {
                    // COMMENT LINE
                    return self.next_value();
                }
                if let Ok(s) = String::from_utf8(line) {
                    if s.is_ascii() {
                        return Some(s)
                    }
                }
            }
        }
        None
    }

    pub fn read_header(&mut self) -> Result<Header, Error> {
        assert_eq!(self.state, State::Signature);

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
            
            if width.is_some() && height.is_some() &
                & depth.is_some() && maxval.is_some() 
                && tupltype.is_some() {
                break;
            }

            match self.next_value() {
                Some(val) => match val.as_ref() {
                    "WIDTH" => {
                        assert_eq!(width.is_none(), true);
                        match self.next_value() {
                            Some(val) => {
                                if let Ok(v) = val.parse::<u64>() {
                                    width = Some(v);
                                } else {
                                    return Err(Error::InvalidHeader);
                                }
                            },
                            None => return Err(Error::InvalidHeader),
                        }
                    },
                    "HEIGHT" => {
                        match self.next_value() {
                            Some(val) => {
                                assert_eq!(height.is_none(), true);
                                if let Ok(v) = val.parse::<u64>() {
                                    height = Some(v);
                                } else {
                                    return Err(Error::InvalidHeader);
                                }
                            },
                            None => return Err(Error::InvalidHeader),
                        }
                    },
                    "DEPTH" => {
                        match self.next_value() {
                            Some(val) => {
                                assert_eq!(depth.is_none(), true);
                                if let Ok(v) = val.parse::<u8>() {
                                    depth = Some(v);
                                } else {
                                    return Err(Error::InvalidHeader);
                                }
                            },
                            None => return Err(Error::InvalidHeader),
                        }
                    },
                    "MAXVAL" => {
                        match self.next_value() {
                            Some(val) => {
                                assert_eq!(maxval.is_none(), true);
                                if let Ok(v) = val.parse::<u16>() {
                                    maxval = Some(v);
                                } else {
                                    return Err(Error::InvalidHeader);
                                }
                            },
                            None => return Err(Error::InvalidHeader),
                        }
                    },
                    "TUPLTYPE" => {
                        match self.next_value() {
                            Some(val) => {
                                assert_eq!(tupltype.is_none(), true);
                                if let Ok(v) = val.parse::<Color>() {
                                    tupltype = Some(v);
                                } else {
                                    return Err(Error::InvalidHeader);
                                }
                            },
                            None => return Err(Error::InvalidHeader),
                        }
                    },
                    "ENDHDR" => {
                        break;
                    },
                    _ => {
                        return Err(Error::InvalidHeader)
                    }
                },
                None => return Err(Error::InvalidHeader),
            }
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

        let pos = self.line_reader.handle.seek(SeekFrom::Current(0)).unwrap();

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
                assert_eq!(signature, PAM_BINARY_MAGIC_NUMBER);
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

