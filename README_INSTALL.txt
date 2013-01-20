

Building on Windows:
	1.	Uncomment the line at the beginning of lightningbeam.py that says "Uncomment to build on Windows"
	2.	In a CMD window, type:
			c:\Python27\python.exe setup.py py2exe
	4.	The executable is in dist\lightningbeam.exe. You probably do need most of the DLL's, I haven't
		gone through them yet.

Building on Ubuntu/Debian:
	1.	In a terminal, type:
			./mkdebian
	2.	This will create a .deb package, a RPM package, and a TGZ package.
Building on Mac OSX:
	1.	In a terminal, type:
			/usr/bin/python setup.py py2app
	2.	This will create an .app package.
