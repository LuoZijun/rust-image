#![feature(try_from, const_fn, duration_as_u128, nll)]
#![allow(unused_variables, unused_imports)]

extern crate crc;
extern crate flate2;
extern crate byteorder;
extern crate num_cpus;

use byteorder::{NetworkEndian, ReadBytesExt};


use std::io;
use std::mem;
use std::cmp;
use std::thread;
use std::convert::TryFrom;
use std::fs::{ File, OpenOptions };
use std::time::{ Duration, Instant };
use std::io::{ Read, Write, Seek, SeekFrom };


/*

PNG Specification, version 1.0:
    http://www.libpng.org/pub/png/spec/1.0/
PNG Specification, version 1.1:
    http://www.libpng.org/pub/png/spec/1.1/
PNG Specification, version 1.2 (includes Gamma and Color tutorials):
    http://www.libpng.org/pub/png/spec/1.2/

Portable Network Graphics (PNG) Specification (Second Edition):
    https://www.w3.org/TR/PNG/#2-ISO-3309

PNG Extensions and Register:
    http://www.libpng.org/pub/png/spec/register/

*/

#[derive(Debug)]
pub enum Error {
    IoError(io::Error),
    Format(&'static str),
    InvalidSignature,
    InvalidChunk,
    CrcMismatch {
        /// bytes to skip to try to recover from this error
        recover: usize,
        /// Stored CRC32 value
        crc_val: u32,
        /// Calculated CRC32 sum
        crc_sum: u32,
        chunk_kind: ChunkKind
    },
    Other(&'static str),
    CorruptFlateStream,
}

impl From<io::Error> for Error {
    fn from(ioerr: io::Error) -> Error {
        Error::IoError(ioerr)
    }
}



pub const VERSION: &'static str = "2.0";

/// [PNG file signature](http://www.w3.org/TR/PNG/#5PNG-file-signature)
pub const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Color {
    Greyscale = 0,
    /// RGB
    Truecolour = 2,
    Indexed = 3,
    GreyscaleWithAlpha = 4,
    /// RGBA
    TruecolourWithAlpha = 6,
}

impl<'a> TryFrom<&'a u8> for Color {
    type Error = ();

    fn try_from(n: &u8) -> Result<Color, Self::Error> {
        match *n {
            0 => Ok(Color::Greyscale),
            2 => Ok(Color::Truecolour),
            3 => Ok(Color::Indexed),
            4 => Ok(Color::GreyscaleWithAlpha),
            6 => Ok(Color::TruecolourWithAlpha),
            _ => Err(()),
        }
    }
}

impl<'a> Into<u8> for &'a Color {
    #[inline]
    fn into(self) -> u8 {
        match *self {
            Color::Greyscale => 0,
            Color::Truecolour => 2,
            Color::Indexed => 3,
            Color::GreyscaleWithAlpha => 4,
            Color::TruecolourWithAlpha => 6,
        }
    }
}

impl TryFrom<u8> for Color {
    type Error = ();

    fn try_from(n: u8) -> Result<Color, Self::Error> {
        Color::try_from(&n)
    }
}

impl Into<u8> for Color {
    fn into(self) -> u8 {
        (&self).into()
    }
}

impl Color {
    /// Returns the number of samples used per pixel of `ColorType`
    pub fn samples(&self) -> usize {
        use self::Color::*;

        match *self {
            Greyscale | Indexed => 1,
            Truecolour => 3,
            GreyscaleWithAlpha => 2,
            TruecolourWithAlpha => 4
        }
    }
}

/// Bit depth of the png file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BitDepth {
    One     = 1,
    Two     = 2,
    Four    = 4,
    Eight   = 8,
    Sixteen = 16,
}

impl<'a> TryFrom<&'a u8> for BitDepth {
    type Error = ();

    fn try_from(n: &u8) -> Result<BitDepth, Self::Error> {
        match *n {
            1 => Ok(BitDepth::One),
            2 => Ok(BitDepth::Two),
            4 => Ok(BitDepth::Four),
            8 => Ok(BitDepth::Eight),
            16 => Ok(BitDepth::Sixteen),
            _ => Err(()),
        }
    }
}

impl<'a> Into<u8> for &'a BitDepth {
    #[inline]
    fn into(self) -> u8 {
        match *self {
            BitDepth::One => 1,
            BitDepth::Two => 2,
            BitDepth::Four => 4,
            BitDepth::Eight => 8,
            BitDepth::Sixteen => 16,
        }
    }
}

impl TryFrom<u8> for BitDepth {
    type Error = ();

    fn try_from(n: u8) -> Result<BitDepth, Self::Error> {
        BitDepth::try_from(&n)
    }
}

impl Into<u8> for BitDepth {
    fn into(self) -> u8 {
        (&self).into()
    }
}


// https://www.w3.org/TR/PNG/#4Concepts.FormatTypes
#[allow(non_upper_case_globals, non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ChunkKind {
    // -- Critical chunks --
    /// image header, which is the first chunk in a PNG datastream.
    IHDR,
    /// palette table associated with indexed PNG images.
    PLTE,
    /// image data chunks.
    IDAT,
    /// image trailer, which is the last chunk in a PNG datastream.
    IEND,

    // -- Ancillary chunks --
    /// Transparency information
    tRNS,
    
    // Colour space information
    /// Primary chromaticities and white point
    cHRM,
    /// Image gamma
    gAMA,
    /// Embedded ICC profile
    iCCP,
    /// Significant bits
    sBIT,
    /// Standard RGB colour space
    sRGB,
    
    // Textual information
    /// International textual data
    iTXt,
    /// Textual data
    tEXt,
    /// Compressed textual data
    zTXt,

    // Miscellaneous information
    /// Background colour
    bKGD,
    /// Image histogram
    hIST,
    /// Physical pixel dimensions
    pHYs,
    /// Suggested palette
    sPLT,

    // Time information
    /// Image last-modification time
    tIME,
    
    // // -- Extension chunks --
    // /// Animation control
    // acTL,
    // /// Frame control
    // fcTL,
    // /// Frame data
    // fdAT,
}

impl<'a> TryFrom<&'a [u8]> for ChunkKind {
    type Error = ();

    fn try_from(bytes: &[u8]) -> Result<ChunkKind, Self::Error> {
        if bytes.len() < 4 {
            return Err(());
        }
        match &bytes[..4] {
            b"IHDR" => Ok(ChunkKind::IHDR),
            b"PLTE" => Ok(ChunkKind::PLTE),
            b"IDAT" => Ok(ChunkKind::IDAT),
            b"IEND" => Ok(ChunkKind::IEND),
            b"tRNS" => Ok(ChunkKind::tRNS),

            b"cHRM" => Ok(ChunkKind::cHRM),
            b"gAMA" => Ok(ChunkKind::gAMA),
            b"iCCP" => Ok(ChunkKind::iCCP),
            b"sBIT" => Ok(ChunkKind::sBIT),
            b"sRGB" => Ok(ChunkKind::sRGB),

            b"iTXt" => Ok(ChunkKind::iTXt),
            b"tEXt" => Ok(ChunkKind::tEXt),
            b"zTXt" => Ok(ChunkKind::zTXt),

            b"bKGD" => Ok(ChunkKind::bKGD),
            b"hIST" => Ok(ChunkKind::hIST),
            b"pHYs" => Ok(ChunkKind::pHYs),
            b"sPLT" => Ok(ChunkKind::sPLT),
            
            b"tIME" => Ok(ChunkKind::tIME),

            // b"acTL" => Ok(ChunkKind::acTL),
            // b"fcTL" => Ok(ChunkKind::fcTL),
            // b"fdAT" => Ok(ChunkKind::fdAT),
            _ => Err(()),
        }
    }
}

impl TryFrom<[u8; 4]> for ChunkKind {
    type Error = ();

    fn try_from(bytes: [u8; 4]) -> Result<ChunkKind, Self::Error> {
        ChunkKind::try_from(&bytes[..])
    }
}

impl<'a> TryFrom<&'a [u8; 4]> for ChunkKind {
    type Error = ();

    fn try_from(bytes: &[u8; 4]) -> Result<ChunkKind, Self::Error> {
        ChunkKind::try_from(&bytes[..])
    }
}

impl<'a> Into<&'static [u8; 4]> for &'a ChunkKind {
    #[inline]
    fn into(self) -> &'static [u8; 4] {
        match *self {
            ChunkKind::IHDR => b"IHDR",
            ChunkKind::PLTE => b"PLTE",
            ChunkKind::IDAT => b"IDAT",
            ChunkKind::IEND => b"IEND",
            ChunkKind::tRNS => b"tRNS",

            ChunkKind::cHRM => b"cHRM",
            ChunkKind::gAMA => b"gAMA",
            ChunkKind::iCCP => b"iCCP",
            ChunkKind::sBIT => b"sBIT",
            ChunkKind::sRGB => b"sRGB",

            ChunkKind::iTXt => b"iTXt",
            ChunkKind::tEXt => b"tEXt",
            ChunkKind::zTXt => b"zTXt",

            ChunkKind::bKGD => b"bKGD",
            ChunkKind::hIST => b"hIST",
            ChunkKind::pHYs => b"pHYs",
            ChunkKind::sPLT => b"sPLT",
            
            ChunkKind::tIME => b"tIME",

            // ChunkKind::acTL => b"acTL",
            // ChunkKind::fcTL => b"fcTL",
            // ChunkKind::fdAT => b"fdAT",
        }
    }
}

impl Into<&'static [u8; 4]> for ChunkKind {
    fn into(self) -> &'static [u8; 4] {
        (&self).into()
    }
}

impl ChunkKind {

    pub fn is_critical_chunk(&self) -> bool {
        match *self {
            ChunkKind::IHDR 
            | ChunkKind::PLTE 
            | ChunkKind::IDAT 
            | ChunkKind::IEND => true,
            _ => false,
        }
    }

    pub fn is_ancillary_chunk(&self) -> bool {
        !self.is_critical_chunk()
    }

}



#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Signature,
    HeaderChunk,
    Chunk(ChunkKind),
    TrailerChunk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    Signature([u8; 8]),
    Chunk(Chunk),
}

impl Element {
    pub fn is_signature(&self) -> bool {
        match *self {
            Element::Signature(_) => true,
            _ => false,
        }
    }

    pub fn is_chunk(&self) -> bool {
        !self.is_signature()
    }

    pub fn signature(&self) -> [u8; 8] {
        match *self {
            Element::Signature(signature) => signature,
            _ => unreachable!(),
        }
    }

    pub fn chunk(&self) -> Chunk {
        match *self {
            Element::Chunk(chunk) => chunk,
            _ => unreachable!(),
        }
    }
}


// https://www.w3.org/TR/PNG/#5DataRep
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub index: usize,
    pub length: u32,
    pub kind: ChunkKind,
    pub crc: [u8; 4],
    pub offset: u64,
}





#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub bitdepth: BitDepth,
    pub color: Color,
    pub compression_method: u8, // 0: deflate/inflate
    pub filter_method: u8,      // 0: adaptive filtering with five basic filter types
    pub interlace_method: u8,   // 0: no interlace  1: Adam7 interlace
}

pub struct Decoder<Handle: Read + Seek> {
    state: State,
    handle: Handle,
    chunk_index: usize,
}

impl<Handle: Read + Seek> Decoder<Handle> {

    pub fn new(handle: Handle) -> Self {
        Decoder {
            state: State::Pending,
            handle: handle,
            chunk_index: 0usize,
        }
    }
    
    pub fn read_signature(&mut self) -> Result<[u8; 8], Error> {
        let mut signature = [0u8; 8];
        
        self.chunk_index = 0usize;

        self.handle.seek(SeekFrom::Start(0)).unwrap();

        match self.handle.read_exact(&mut signature) {
            Ok(_) => {
                self.state = State::Signature;
                Ok(signature)
            },
            Err(io_error) => Err(io_error.into())
        }
    }

    pub fn read_chunk(&mut self) -> Result<Chunk, Error> {
        let length: u32 = self.handle.read_u32::<NetworkEndian>().unwrap();

        let mut buf = [0u8; 4];

        let kind: ChunkKind = {
            if let Ok(_) = self.handle.read_exact(&mut buf) {
                if let Ok(chunk_kind) = ChunkKind::try_from(&buf) {
                    chunk_kind
                } else {
                    return Err(Error::InvalidChunk);
                }
            } else {
                return Err(Error::InvalidChunk);
            }
        };

        let pos: u64 = self.handle.seek(SeekFrom::Current(0)).unwrap();

        self.handle.seek(SeekFrom::Current(length as i64)).unwrap();

        self.handle.read_exact(&mut buf).unwrap();
        let crc: [u8; 4] = buf;

        let chunk = Chunk {
            index: self.chunk_index,
            length: length,
            kind: kind,
            crc: crc,
            offset: pos,
        };

        self.chunk_index += 1;
        self.state = State::Chunk(kind);

        Ok(chunk)
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
        } else if self.state == State::Chunk(ChunkKind::IEND) {
            None
        } else {
            if let Ok(chunk) = self.read_chunk() {
                Some(Element::Chunk(chunk))
            } else {
                None
            }
        }
    }
}



fn main(){
    let core = num_cpus::get_physical();
    let threads = num_cpus::get();

    println!("Core: {:?} Threads: {:?}", core, threads);

    // let filepath = "/Users/luozijun/Pictures/qmshtu.png";
    let filepath = "output.png";

    let mut file = File::open(filepath).unwrap();

    let decoder = Decoder::new(file.try_clone().unwrap());
    let chunks: Vec<Chunk> = decoder.filter(|elem| elem.is_chunk())
                                    .map(|elem| elem.chunk())
                                    .collect::<Vec<Chunk>>();

    pub const CHUNCK_BUFFER_SIZE: usize = 4 * 1024;

    let mut buffer: [u8; CHUNCK_BUFFER_SIZE] = [0u8; CHUNCK_BUFFER_SIZE];
    let mut zlib_decoder = flate2::write::ZlibDecoder::new(Vec::new());

    let now = Instant::now();

    let mut header: Option<Header> = None;

    for chunk in chunks.iter() {

        if chunk.kind == ChunkKind::IHDR {
            assert_eq!(header.is_none(), true);

            file.seek(SeekFrom::Start(chunk.offset)).unwrap();
            
            pub const HEADER_SIZE: u32 = 13u32;

            assert_eq!(chunk.length, HEADER_SIZE as u32);

            let width: u32 = file.read_u32::<NetworkEndian>().unwrap();
            let height: u32 = file.read_u32::<NetworkEndian>().unwrap();

            assert_eq!(file.read(&mut buffer[..1]).unwrap(), 1);
            let bitdepth: BitDepth = BitDepth::try_from(buffer[0]).unwrap();

            assert_eq!(file.read(&mut buffer[..1]).unwrap(), 1);
            let color: Color = Color::try_from(buffer[0]).unwrap();

            assert_eq!(file.read(&mut buffer[..1]).unwrap(), 1);
            let compression_method = buffer[0];

            assert_eq!(file.read(&mut buffer[..1]).unwrap(), 1);
            let filter_method = buffer[0];

            assert_eq!(file.read(&mut buffer[..1]).unwrap(), 1);
            let interlace_method = buffer[0];
            
            header = Some(Header {
                width: width,
                height: height,
                bitdepth: bitdepth,
                color: color,
                compression_method: compression_method,
                filter_method: filter_method,
                interlace_method: interlace_method,
            });

            continue;
        }

        if chunk.kind != ChunkKind::IDAT {
            continue
        }
        
        println!("{:?}", chunk);

        let size = chunk.length as u64;
        let mut readed = 0u64;

        file.seek(SeekFrom::Start(chunk.offset)).unwrap();

        while readed < size {
            let amt = file.read(&mut buffer).unwrap();
            
            if amt == 0 {
                break;
            }

            zlib_decoder.write(&buffer[..amt]).unwrap();

            let pixels = zlib_decoder.get_ref().len() as f64 / 3.0;
            let ms = (now.elapsed().as_millis() as f64) / 1000.0;
            let pps = pixels / ms;
            println!("Chunk: {:?} decoded {:?} pixels  elapsed: {:?} seconds  PPS: {:?}", chunk.index, pixels, ms, pps);
            readed += amt as u64;
        }
    }

    println!("{:?}", header);
    let pixels = &zlib_decoder.finish().unwrap()[1..];
    // println!("{:?}", pixels);
    println!("Pixels: {:?} Bytes", pixels.len() );
}

