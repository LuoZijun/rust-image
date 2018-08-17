实验性的图像库
=================

:Date: 08/17 2018

.. contents::

介绍
----------

一个使用 Rust 编写的图像解码库，仅供研究学习使用。

格式支持
----------

*   PNG
*   PAM
*   PPM(TODO)

测试
----------

.. code:: bash

    git clone https://github.com/LuoZijun/rust-image.git
    cd rust-image
    curl "https://avatars0.githubusercontent.com/u/717717" -o ./output.jpg
    ffmpeg -i output.jpg output.png
    ffmpeg -i output.jpg output.pam

    cargo run --bin png
    cargo run --bin pam



参考
-------

`Portable Network Graphics (PNG) Specification (Second Edition) <https://www.w3.org/TR/PNG/>`_

`Portable Arbitrary Map (PAM) <http://netpbm.sourceforge.net/doc/pam.html>`_

