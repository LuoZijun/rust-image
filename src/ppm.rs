#![feature(try_from, const_fn, duration_as_u128, nll)]
#![allow(unused_variables, unused_imports, unused_mut)]

// http://netpbm.sourceforge.net/doc/ppm.html

mod netpbm;

pub use self::netpbm::{ PPM_ASCII_MAGIC_NUMBER, PPM_BINARY_MAGIC_NUMBER, Lines };

use std::io;
use std::fmt;
use std::mem;
use std::cmp;
use std::str;
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
pub struct Header {
    pub width: u64,
    pub height: u64,
    pub maxval: u16,
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

        let width: u64 = {
            match self.next_value() {
                Some(val) => {
                    if let Ok(v) = val.parse::<u64>() {
                        v
                    } else {
                        return Err(Error::InvalidHeader);
                    }
                },
                None => return Err(Error::InvalidHeader),
            }
        };

        let height: u64 = {
            match self.next_value() {
                Some(val) => {
                    if let Ok(v) = val.parse::<u64>() {
                        v
                    } else {
                        return Err(Error::InvalidHeader);
                    }
                },
                None => return Err(Error::InvalidHeader),
            }
        };

        let maxval: u16 = {
            match self.next_value() {
                Some(val) => {
                    if let Ok(v) = val.parse::<u16>() {
                        v
                    } else {
                        return Err(Error::InvalidHeader);
                    }
                },
                None => return Err(Error::InvalidHeader),
            }
        };

        if maxval < 1 || maxval > 255 {
            return Err(Error::InvalidHeader);
        }

        let header = Header { width, height, maxval };

        // bytes per pixel
        let pixels_size = header.width * header.height * 3;

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


fn main(){
    let filepath = "output.ppm";
    let mut file = File::open(filepath).unwrap();
    let mut decoder = Decoder::new(file.try_clone().unwrap());

    let mut signature: Option<[u8; 2]> = None;

    for elem in decoder {
        match elem {
            Element::Signature(_signature) => {
                println!("Signature: {:?}", _signature);
                assert_eq!(_signature == PPM_BINARY_MAGIC_NUMBER || _signature == PPM_ASCII_MAGIC_NUMBER, true);
                signature = Some(_signature);
            },
            Element::Header(header) => {
                println!("{:?}", header);
            },
            Element::Data(data) => {
                println!("{:?}", data);

                if signature == Some(PPM_BINARY_MAGIC_NUMBER) {
                    let mut pixels: Vec<u8> = vec![0u8; data.length as usize];
                    file.seek(SeekFrom::Start(data.offset)).unwrap();
                    assert_eq!(file.read(&mut pixels).unwrap(), data.length as usize);
                    println!("{:?}", pixels);
                }
                
                println!("Pixel len: {:?} Bytes", data.length);
            },
        }
    }
}