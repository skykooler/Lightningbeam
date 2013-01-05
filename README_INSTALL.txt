

Building on Windows:
	1.	In a CMD window, type:
			c:\Python27\python.exe setup.py py2exe
	2.	Copy the GUI folder from the root directory into dist\.
	3.	The executable is in dist\lightningbeam.exe. You probably do need most of the DLL's, I haven't
		gone through them yet.

Building on Ubuntu/Debian:
	1.	In a terminal, type:
			./mkdebian
			cd debian
			dpkg --build lightningbeam ./
	2.	Now there is a package, which can be installed however.
	3.	To create a RPM package:
			sudo apt-get install alien
			alien -r lightningbeam.deb
	4.	To create a Slackware TGZ package:
			alien -t lightningbeam.deb
Building on Mac OSX:
	1.	In a terminal, type:
			/usr/bin/python setup.py py2app
	2.	This will create an .app package.
